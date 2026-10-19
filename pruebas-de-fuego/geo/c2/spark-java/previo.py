"""ORE 0068 C2·3 pieza 4 · El previo de Spark Java: ¿puede el código Java de un Transform correr en la
JVM de Spark del puesto pesado, con `over()`/`write()` resueltos por el SDK de Python (py4j, al revés)?
Sin red, como el uid de los puestos, en `ore-puesto-spark:local`:
    bash pruebas-de-fuego/geo/c2/spark-java/medir.sh
Una línea `OK|FALLA · caso · detalle`, y al final `n/m casos`.
"""
import glob
import json
import os
import subprocess
import time

SPARK_HOME = os.environ["SPARK_HOME"]
CLS = "/tmp/previo-sdk"
casos = []


def caso(nombre, f, ok):
    t = time.time()
    try:
        r = f()
    except Exception as e:  # noqa: BLE001
        r = "EXC %s: %s" % (type(e).__name__, " | ".join(x.strip() for x in str(e).splitlines() if x.strip())[:600])
    bien = bool(ok(r))
    casos.append(bien)
    print("%s · %s · %.1f s · %s" % ("OK   " if bien else "FALLA", nombre, time.time() - t, r), flush=True)
    return r


# ── 0 · el SDK de previo, compilado contra los jars de Spark (como lo haría la imagen) ──────────────
os.makedirs(CLS, exist_ok=True)
fuentes = glob.glob("/l/sdk/ore/*.java")
subprocess.run(["javac", "-proc:none", "-nowarn", "--release", "21", "-cp", SPARK_HOME + "/jars/*", "-d", CLS] + fuentes, check=True)

from pyspark.sql import DataFrame, SparkSession  # noqa: E402

t0 = time.time()
s = (SparkSession.builder.master("local[2]").config("spark.driver.memory", "1g")
     .config("spark.driver.extraClassPath", CLS).getOrCreate())
print("sesión en %.1f s" % (time.time() - t0))
jvm, gw = s._jvm, s.sparkContext._gateway

# ── 1 · el puente: Python implementa una interfaz de Java ───────────────────────────────────────────
escrito = {}


class Puente:
    def over(self, nombre):
        if nombre == "geo.prohibida":
            raise PermissionError("transform(): `geo.prohibida` is not among the inputs of `zonas`")
        return s.sql("select id, ST_SetSRID(ST_Point(440000 + id * 10, 4474000), 25830) geom from range(5)")._jdf

    def write(self, nombre, datos, modo):
        df = DataFrame(datos, s)
        escrito[nombre] = (df.columns, df.count(), [r.a for r in df.selectExpr("round(ST_Area(zona)) a").distinct().collect()]
                           if "zona" in df.columns else None)
        return json.dumps({"rows": escrito[nombre][1], "mode": modo})

    class Java:
        implements = ["ore.Puente"]


from pyspark.java_gateway import ensure_callback_server_started  # noqa: E402

caso("1 · el servidor de vuelta (callback) arranca", lambda: (ensure_callback_server_started(gw), "ok")[1], lambda r: r == "ok")
jvm.ore.Ore.puente(Puente())
caso("1b · el classpath del driver", lambda: jvm.ore.Ore.classpath()[:200], lambda r: isinstance(r, str))

# ── 2 · javac dentro del driver ─────────────────────────────────────────────────────────────────────
ZONAS = """package geo;

import static ore.Ore.*;

import org.apache.spark.sql.Dataset;
import org.apache.spark.sql.Row;

public class Zonas {
    public static Object zonas() throws Exception {
        Dataset<Row> l = over("geo.lugares");
        return write("geo.zonas", l.selectExpr("id", "ST_Buffer(geom, 100) zona"));
    }

    public static Object rompe() throws Exception {
        Dataset<Row> l = over("geo.lugares");
        Object x = null;
        return x.toString();
    }

    public static Object prohibida() throws Exception {
        return write("geo.zonas", over("geo.prohibida"));
    }

    public static Object malSql() throws Exception {
        return write("geo.zonas", over("geo.lugares").selectExpr("no_existe + 1"));
    }
}
"""
m = {}
caso("2 · javac con el classpath tal cual (comodines)", lambda: str(jvm.ore.Ore.compilar("geo.Zonas", ZONAS, "zonas", False)),
     lambda r: "zonas" in r and not r.startswith("EXC"))
m["zonas"] = caso("2b · javac con los jars expandidos", lambda: jvm.ore.Ore.compilar("geo.Zonas", ZONAS, "zonas", True),
                  lambda r: not isinstance(r, str))
caso("2c · un error de compilación, con su línea", lambda: str(jvm.ore.Ore.compilar("geo.Zonas", ZONAS.replace("Dataset<Row> l", "Datset<Row> l", 1), "zonas", True)),
     lambda r: "CompilationError línea 10" in r)

# ── 3 · llamarlo: over() da el Dataset de Python, write() se lo devuelve ────────────────────────────
caso("3 · el transform: over → selectExpr(ST_Buffer) → write", lambda: (jvm.ore.Ore.llamar(m["zonas"], "Zonas.java"), escrito.get("geo.zonas")),
     lambda r: not isinstance(r, str) and json.loads(r[0])["rows"] == 5 and r[1][2] == [31214.0])
for metodo, espera, nombre in [
    ("rompe", "runtime línea 17: NullPointerException", "3b · un NPE del usuario, con su línea"),
    ("prohibida", "PermissionError", "3c · un PermissionError de Python (el techo), dicho en Java"),
    ("malSql", "no_existe", "3d · un AnalysisException de Spark en el código del usuario"),
]:
    def correr(metodo=metodo):
        mm = jvm.ore.Ore.compilar("geo.Zonas", ZONAS, metodo, True)
        return str(jvm.ore.Ore.llamar(mm, "Zonas.java"))
    caso(nombre, correr, lambda r, e=espera: r.startswith("EXC") and e in r)

# ── 4 · el coste: compilar otra vez (la JVM ya caliente) ────────────────────────────────────────────
caso("4 · compilar en caliente", lambda: (jvm.ore.Ore.compilar("geo.Zonas", ZONAS, "zonas", True), "ok")[1], lambda r: r == "ok")

print("\n%d/%d casos" % (sum(casos), len(casos)))
