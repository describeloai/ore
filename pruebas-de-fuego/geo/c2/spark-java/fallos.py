"""ORE 0068 C2·3 pieza 4 · Los fallos de un build de Java en Spark, con el SDK de verdad (`ore.spark.java`
y `ore-spark-sdk.jar` de la imagen), sin red ni banco: lo que cada uno deja en el informe —tipo,
fichero, línea— y en el mensaje. ore-serve se contesta aquí (el techo se dice y se acepta).
    bash pruebas-de-fuego/geo/c2/spark-java/medir.sh fallos.py
"""
import os
import sys

os.environ.update({"ORE_MOTOR": "heavyweight", "ORE_SERVE": "http://127.0.0.1:9", "ORE_SUJETO": "persona:ana",
                   "ORE_MEMORIA_MB": "3072"})
sys.path.insert(0, "/opt/ore")
import ore  # noqa: E402
from ore import spark as pesado  # noqa: E402

ore.session.pedir = lambda *a, **k: (200, {})
F = "packages/geo/j/transforms/geo/Z.java"
casos = []


def fuente(cuerpo, cabeza=""):
    return ("package geo;\n\nimport static ore.Ore.*;\n\nimport ore.Transform;\nimport org.apache.spark.sql.Dataset;\n"
            "import org.apache.spark.sql.Row;\n\npublic class Z {\n%s"
            "    @Transform(inputs = {\"geo.a\"}, output = \"geo.z\")\n    public static Object z() throws Exception {\n"
            "%s\n    }\n}\n" % (cabeza, cuerpo))


def caso(nombre, texto, espera_tipo, espera_linea, espera_texto, metodo="z", otras=None):
    ore._tomar_informe()
    try:
        pesado.java({"fichero": F, "metodo": metodo, "fuentes": dict({F: texto}, **(otras or {}))})
        r, inf = "no falló", ore._tomar_informe()
    except Exception as e:  # noqa: BLE001
        r, inf = "%s: %s" % (type(e).__name__, str(e).splitlines()[0][:260]), ore._tomar_informe()
    err = (inf or {}).get("error") or {}
    bien = err.get("tipo") == espera_tipo and err.get("linea") == espera_linea and espera_texto in r
    casos.append(bien)
    print("%s · %s · %s · %s" % ("OK   " if bien else "FALLA", nombre, err, r), flush=True)


# líneas: 10 = la anotación, 11 = la firma, 12 = la primera del cuerpo
caso("1 · no compila", fuente("        Datset<Row> d = null;\n        return d;"), "syntax", 12, "CompilationError: cannot find symbol")
caso("1b · no compila una de apoyo (su fichero)", fuente("        return Ayuda.x();"), "syntax", 4, "CompilationError",
     otras={"packages/geo/j/transforms/geo/Ayuda.java": "package geo;\n\nclass Ayuda {\n    static Object x() { return nope; }\n}\n"})
caso("2 · no es un @Transform", fuente("        return null;"), "not-a-transform", None, "is not a `@Transform` method", metodo="otro")
caso("3 · D15: over() al cargar la clase", fuente("        return null;", "    static final Object X = over(\"geo.a\");\n\n"),
     "called-while-loading", 10, "while the build loads the class")
caso("4 · write() fuera del techo (Python lo niega)", fuente(
    "        return write(\"geo.otra\", spark().sql(\"select 1 a\"));"), "runtime", 12, "PermissionError: `geo.otra` is not the output")
caso("5 · una geometría sin CRS (Python lo niega)", fuente(
    "        return write(\"geo.z\", spark().sql(\"select ST_Point(1, 2) g\"));"), "runtime", 12, "ValueError: write(): `g` has no CRS")
caso("6 · un AnalysisException de Spark", fuente(
    "        return write(\"geo.z\", spark().sql(\"select 1 a\").selectExpr(\"no_hay\"));"), "runtime", 12, "UNRESOLVED_COLUMN")
caso("7 · un método que no escribe", fuente("        return spark().sql(\"select 1\").count();"), None, None, "no falló")
print("\n%d/%d casos" % (sum(casos), len(casos)))
