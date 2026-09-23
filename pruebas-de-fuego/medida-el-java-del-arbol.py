"""0037 ③b · EL JAVA DEL ARBOL, MEDIDO ANTES DE DECIDIR NADA.

Python quedo cerrado de punta a punta (③a). Java no, y la medida de ③a dijo por
que en una linea: jdtls da CUATRO ERRORES DE SINTAXIS sobre nuestra propia
semilla, porque la semilla es un SNIPPET DE JSHELL y jdtls analiza UNIDADES DE
COMPILACION. Aqui se mide el resto: que acepta el ejecutor que ya tenemos, que
necesita jdtls para resolver NUESTRO SDK, donde puede vivir el fichero y que
cuesta todo eso.

  §1  QUE ES HOY UN `.java` DEL ARBOL   la semilla, y lo que el agente hace con
                                        ella.
  §2  QUE ACEPTA EL EJECUTOR            medido con un JShell 21 de verdad: la
                                        misma semilla como fichero de
                                        compilacion.
  §3  QUE NECESITA jdtls                para resolver `ore.Ore`, medido en
                                        cuatro escenarios.
  §4  DONDE PUEDE VIVIR EL FICHERO      la carpeta, el paquete y el nombre.
  §5  QUE CUESTA                        disco, arranque y memoria, contra lo
                                        que el pod pide.
  §6  ¿UNA JVM O DOS?                   la pregunta que ③b existe para
                                        contestar: que sabe hacer YA la JVM del
                                        agente, y que anadiria una segunda.

    python pruebas-de-fuego/medida-el-java-del-arbol.py
"""
import os
import re
import sys

sys.stdout.reconfigure(encoding="utf-8", errors="replace")
AQUI = os.path.dirname(os.path.abspath(__file__))
RAIZ = os.path.dirname(AQUI)

# ─────────────────────────────────────────────────────────────────────────────
# Medido el 2026-09-23 con jdtls 1.57.0 sobre un Temurin 21 COMPLETO y un
# cliente LSP de verdad, contra un fichero con la forma que tendria la semilla
# —`import static ore.Ore.*;` + `public class Ejemplo` con `main`— y un SDK de
# mentira con las tres firmas (`over`, `write`, `transform`) y su javadoc.
#
#   a) sin decirle donde esta el SDK
#   b) con el SDK como DIRECTORIO de clases en `referencedLibraries`
#   c) con el SDK como JAR
#   d) con el JAR y su `-sources.jar` al lado
#   e) con el JAR, pero el fichero renombrado y la clase no
# ─────────────────────────────────────────────────────────────────────────────
ESCENARIOS = [
    ("sin decirle donde esta el SDK", "2 errores: «The import ore cannot be resolved» y "
     "«The method transform(...) is undefined»", "NINGUNA", "vacio", 573),
    ("el SDK como DIRECTORIO de clases", "LOS MISMOS 2 ERRORES: un directorio no vale", "NINGUNA", "vacio", 561),
    ("el SDK como JAR", "0 diagnosticos", "write(String, Object)", "la firma, sin prosa", 851),
    ("el JAR y su `-sources.jar` al lado", "0 diagnosticos", "write(String nombre, Object datos)",
     "la firma Y NUESTRO JAVADOC", 580),
    ("el fichero renombrado y la clase no", "1 error: «The public type Ejemplo must be defined in its own file»",
     "NINGUNA", "sigue resolviendo", 493),
]
ARRANQUE = "5,6-6,6 s hasta `ServiceReady` (22,2 s la primerisima vez, con el disco frio)"

# ─────────────────────────────────────────────────────────────────────────────
# §6 · medido el 2026-09-23 en UNA SOLA JVM (la que el agente ya abre), con el
# mismo JDK 21 y el mismo SDK de mentira. `completionSuggestions` y
# `documentation` son del API que el agente YA USA para partir la celda en
# snippets (`SourceCodeAnalysis`); `javac` es el compilador del propio JDK,
# llamado en proceso con el texto del editor en memoria (sin fichero en disco).
#
# Y la memoria del agente DE VERDAD sale de la prueba de fuego entera, muestreada
# cada 3 s mientras el caso 9 corre Java contra agentes reales (`over()`, `sql()`
# y `write()` con DuckDB y Arrow dentro).
# ─────────────────────────────────────────────────────────────────────────────
UNA_JVM = {
    "jshell_listo_ms": 992,
    "memoria_jshell_mb": 108,
    "completion": [("wr", 1, 232, "write("), ("ov", 1, 12, "over("),
                   ("escrito.", 32, 68, "clear(), compute(, containsKey(…"),
                   ("String.fo", 2, 25, "format(, format(")],
    "documentation": "la FIRMA de lo nuestro (`Map<String,Object> ore.Ore.write(String, Object)`) "
                     "y el javadoc ENTERO del JDK; el javadoc NUESTRO no",
    "javac_frio_ms": 249, "javac_caliente_ms": 68,
    "javac_dice": "linea 12:39 · ERROR · cannot find symbol  symbol: variable escrit",
    "memoria_total_mb": (249, 270),
}
AGENTE_REAL_MB = (140, 265)
COMPLETION_S = 0.61
TAMANO_MB = 53


def titulo(t):
    print("\n" + t)
    print("  " + "-" * (len(t) - 2))


def leer(*p):
    return open(os.path.join(*p), encoding="utf-8", errors="replace").read()


def seccion_que_es_hoy():
    titulo("  §1 QUE ES HOY UN `.java` DEL ARBOL")
    c = leer(RAIZ, "crates", "ore-core", "src", "clases.rs")
    i = c.index("const TRANSFORMS_JAVA")
    semilla = c[c.index('"', i) + 1:c.index('\n";', i)]
    lineas = [l for l in semilla.splitlines()
              if l.strip() and l.strip() != chr(92) and not l.strip().startswith("//")]
    print("     la semilla de `transforms-java`, sin comentarios:")
    for l in lineas:
        print("       %s" % l.replace('\\"', '"'))
    print("     ⇒ Eso NO es un fichero Java: no hay clase, hay `var` en el tope y")
    print("       llamadas sueltas. Es un SNIPPET DE JSHELL, que es lo que la sesion")
    print("       de la JVM evalua.")
    a = leer(RAIZ, "puesto", "jvm", "ore", "Agente.java")
    llama_main = "static void main(" in a and ".main(new String[0]);" in a
    print("")
    print("     Y EL EJECUTOR YA SABE MAS DE LO QUE LA SEMILLA USA: el agente parte")
    print("     la celda en snippets (`analyzeCompletion`) y, si declara una clase con")
    print("     `static void main(`, LA LLAMA: %s" % ("sí" if llama_main else "no"))
    print("       (lo dice su propio comentario: «Un `.java` del arbol con `main`: se")
    print("        llama, como `java Fichero.java`»)")


def seccion_que_acepta():
    titulo("  §2 QUE ACEPTA EL EJECUTOR (medido con un JShell 21 de verdad)")
    print("     Se le dio, por la entrada estandar, un FICHERO DE COMPILACION con la")
    print("     forma que tendria la semilla:")
    print("")
    print("       import static ore.Ore.*;")
    print("       public class Ejemplo {")
    print("           public static void main(String[] args) { ... transform(...) ... }")
    print("       }")
    print("")
    print("     jshell --class-path <sdk>  ⇒  imprime `filas 0` y NI UN ERROR.")
    print("     ⭐ Es decir: la semilla puede pasar a ser un fichero Java de verdad")
    print("       SIN TOCAR EL EJECUTOR. JShell acepta el `public`, acepta el import")
    print("       estatico, y el agente llama al `main` el solo.")
    print("     ⛔ Lo unico que JShell NO acepta es `package ...;` — y la semilla no")
    print("       lo lleva ni lo necesita (§4).")


def seccion_jdtls():
    titulo("  §3 QUE NECESITA jdtls PARA RESOLVER NUESTRO SDK")
    print("     %-38s %-52s %s" % ("escenario", "diagnosticos", "memoria"))
    for nombre, diag, _, _, mem in ESCENARIOS:
        print("     %-38s %-52s %d MB" % (nombre, diag[:52], mem))
    print("")
    print("     y lo que ofrece al teclear `wr` dentro del metodo:")
    for nombre, _, comp, hover, _ in ESCENARIOS:
        print("       %-38s completion: %-34s hover: %s" % (nombre, comp, hover))
    print("")
    print("     ⛔⛔ UN DIRECTORIO DE CLASES NO VALE, Y ESTO MANDA EN LA IMAGEN: hoy el")
    print("       Dockerfile compila el SDK a `/opt/ore/clases` y arranca con")
    print("       `-cp /opt/ore/clases:/opt/ore/lib/*`. Con eso, jdtls dice «The import")
    print("       ore cannot be resolved» IGUAL que si no le dijeras nada. Con el mismo")
    print("       SDK empaquetado en un `ore.jar`: cero diagnosticos.")
    print("     ⭐ Y EL JAVADOC VIAJA EN OTRO JAR: `ore-sources.jar` al lado, que jdtls")
    print("       coge POR EL NOMBRE sin que nadie se lo diga. Sin el, el hover dice la")
    print("       firma; con el, dice lo que la funcion hace — que es lo que se nota.")
    print("     completion: %.2f s" % COMPLETION_S)


def seccion_donde_vive():
    titulo("  §4 DONDE PUEDE VIVIR EL FICHERO")
    print("     · en la raiz del repositorio y en `transforms/`: IGUAL de bien, y sin")
    print("       declarar `package`. jdtls monta un «proyecto invisible» y no exige")
    print("       que la carpeta coincida con nada.")
    print("     · el nombre del fichero ES el nombre de la clase publica. Medido:")
    print("       renombrar `Ejemplo.java` a `Otro.java` sin renombrar la clase da")
    print("       «The public type Ejemplo must be defined in its own file».")
    print("     ⇒ Dos reglas que hoy no existen en el arbol y que habria que decir en")
    print("       algun sitio: sin `package`, y el nombre manda.")


def seccion_coste():
    titulo("  §5 QUE CUESTA")
    y = leer(RAIZ, "malla", "51-el-puesto.yaml")
    nombre, pide, tope = None, None, None
    for linea in y.splitlines():
        m = re.search(r"- name: ([a-z0-9-]+)\s*$", linea)
        if m:
            nombre = m.group(1)
        r = re.search(r'requests: \{cpu: "([^"]+)", memory: ([^\}]+)\}', linea)
        if r and nombre == "puesto":
            pide = (r.group(1), r.group(2).strip())
        l = re.search(r'limits:\s+\{cpu: "([^"]+)", memory: ([^\}]+)\}', linea)
        if l and nombre == "puesto":
            tope = (l.group(1), l.group(2).strip())
    print("     jdtls en disco: %d MB (y el JDK ya esta en la imagen)" % TAMANO_MB)
    print("     arranque: %s" % ARRANQUE)
    print("     memoria: entre %d y %d MB segun lo que tenga que indexar"
          % (min(e[4] for e in ESCENARIOS), max(e[4] for e in ESCENARIOS)))
    if pide:
        print("     el contenedor `puesto` pide cpu %s / mem %s (tope cpu %s / mem %s)"
              % (pide[0], pide[1], tope[0] if tope else "?", tope[1] if tope else "?"))
    print("     ⇒ Y ahi dentro ya corre OTRA JVM: la del agente, con JShell. Dos JVM en")
    print("       el mismo pod es la decision de tamano que Java trae y Python no:")
    print("       pyright pedia 210 MB, jdtls pide el doble o el cuadruple.")
    print("     ⇒ Comparado con Python: pyright arranca en 0,5 s y contesta el primer")
    print("       diagnostico en 3,8 s; jdtls tarda ~6 s solo en decir que esta listo, y")
    print("       ANTES DE ESO un `didOpen` devuelve CERO diagnosticos y el fichero")
    print("       parece correcto (medido en ③a). Quien lo hable tiene que esperar su")
    print("       `language/status ServiceReady`.")


def seccion_una_o_dos():
    titulo("  §6 ¿UNA JVM O DOS?")
    print("     Por que serian dos: jdtls es una aplicacion de Eclipse (OSGi/Equinox)")
    print("     que habla LSP por su entrada y su salida — un proceso aparte por")
    print("     diseno—, y el agente ya es otra JVM con JShell dentro.")
    print("")
    print("     LO QUE PIDE CADA UNA:")
    print("       el agente, medido en la prueba de fuego mientras corre Java de")
    print("       verdad (over, sql y write con DuckDB y Arrow): %d-%d MB" % AGENTE_REAL_MB)
    print("       jdtls, al lado: %d-%d MB mas, en otro proceso"
          % (min(e[4] for e in ESCENARIOS), max(e[4] for e in ESCENARIOS)))
    print("")
    print("     ⭐⭐ PERO LA JVM DEL AGENTE YA SABE HACER LA MAYOR PARTE, y con el API")
    print("       QUE YA USA. Medido en un solo proceso:")
    print("       JShell listo en %d ms (memoria %d MB)"
          % (UNA_JVM["jshell_listo_ms"], UNA_JVM["memoria_jshell_mb"]))
    for texto, n, ms, muestra in UNA_JVM["completion"]:
        print("         completion `%-9s` %3d propuestas en %3d ms · %s" % (texto, n, ms, muestra))
    print("       documentation: %s" % UNA_JVM["documentation"])
    print("       javac EN PROCESO, con el texto del editor en memoria:")
    print("         %d ms en frio, %d ms CALIENTE — que es lo que costaria cada vez"
          % (UNA_JVM["javac_frio_ms"], UNA_JVM["javac_caliente_ms"]))
    print("         y lo que dice: «%s»" % UNA_JVM["javac_dice"])
    print("       memoria del proceso entero al acabar: %d-%d MB" % UNA_JVM["memoria_total_mb"])
    print("")
    print("     ⇒ DIAGNOSTICOS Y AUTOCOMPLETADO SALEN SIN SEGUNDA JVM, sin 53 MB de")
    print("       imagen y sin 6 s de arranque. Lo que se pierde es lo que jdtls hace")
    print("       ADEMAS: ir a la definicion, renombrar, buscar referencias, arreglos")
    print("       rapidos, organizar imports, y ver el repositorio ENTERO (javac ve el")
    print("       fichero que se le da).")
    print("     ⛔ Y una cosa que hoy NO sale por ningun lado: NUESTRO javadoc. JShell")
    print("       lo saca del `src.zip` del JDK para lo suyo, pero de un jar del")
    print("       classpath no lo lee; jdtls SI, si al lado del jar va un")
    print("       `ore-sources.jar`. Es la unica ventaja de jdtls que se nota al")
    print("       escribir — y es una que podriamos dar nosotros, que para eso")
    print("       conocemos las nueve funciones del SDK.")


def main():
    print("=== 0037 ③b · el Java del arbol, medido")
    seccion_que_es_hoy()
    seccion_que_acepta()
    seccion_jdtls()
    seccion_donde_vive()
    seccion_coste()
    seccion_una_o_dos()
    return 0


if __name__ == "__main__":
    sys.exit(main())
