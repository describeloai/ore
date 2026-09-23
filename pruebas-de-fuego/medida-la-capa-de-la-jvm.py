"""0037 ③c · LA CAPA DE LA JVM, MEDIDA ANTES DE CAVARLA.

Hoy un repositorio de Java NO PUEDE USAR NI UNA BIBLIOTECA DE TERCEROS. No es
una limitación del lenguaje: es que la capa —lo que el árbol declara, resuelto
en un sitio y montado en la sesión (0031 W3.2, 0036 ③)— sólo entiende
`pyproject.toml`. Esta medida dice qué hay, qué falta y qué costaría.

  §1  LO QUE LA CAPA ENTIENDE HOY   leído del código y de la plantilla del Job.
  §2  POR QUE NO VALE PARA LA JVM   lo que hace `pip download` y lo que haría
                                    falta para jars.
  §3  QUIEN RESUELVE                Maven y coursier, medidos: tamaño y una
                                    resolución de verdad.
  §4  DONDE CORRERIA                qué imagen tiene ya lo que hace falta.
  §5  LA FORMA QUE SALE             qué declara el repositorio, y por qué ESE
                                    fichero y no uno nuestro.
  §6  EL CHOQUE                     lo que la imagen YA lleva, y qué pasa
                                    cuando la capa trae otra versión.
  §7  COMO SE EVITA                 medido: `provided` + `includeScope=runtime`.
  §8  REPETIBLE Y SIN RED           frío, caliente y offline, con los números.
  §9  LOS BORDES                    lo que queda por decidir, dicho antes de
                                    construir.

    python pruebas-de-fuego/medida-la-capa-de-la-jvm.py
"""
import os
import re
import sys

sys.stdout.reconfigure(encoding="utf-8", errors="replace")
AQUI = os.path.dirname(os.path.abspath(__file__))
RAIZ = os.path.dirname(AQUI)

# ─────────────────────────────────────────────────────────────────────────────
# Medido el 2026-09-23. Los tamaños, de la cabecera HTTP de la descarga; la
# resolución, corriendo Maven de verdad con un JDK 21 y un repositorio local
# VACÍO —que es exactamente lo que el Job tendría la primera vez— sobre un
# `pom.xml` con una sola dependencia transitiva de las nuestras.
# ─────────────────────────────────────────────────────────────────────────────
RESOLUTORES = {
    "Maven 3.9.9 (binario)": {"mb": 9.1, "desplegado_mb": 11, "pide": "un JDK"},
    "coursier (lanzador jar)": {"mb": 0.23, "desplegado_mb": 0.23, "pide": "un JDK"},
    "coursier (binario nativo linux)": {"mb": 29.7, "desplegado_mb": 29.7, "pide": "nada"},
}
MAVEN = {"dependencia": "org.apache.arrow:arrow-vector:19.0.0",
         "jars": 10, "mb": 4.8, "segundos": 46.5}
# Lo que la capa de Python cuesta hoy, de la cabecera de `52-la-capa.yaml`.
PYTHON = "polars: resolver y bajar las ruedas, subir 51 MB en 1 s; el puesto las baja en 0,9 s"

# ── §6-§8 · medido el 2026-09-23 con Maven 3.9.9 y un JDK 21 ────────────────
#
# El choque: dos clases con el mismo nombre en dos sitios del classpath.
ORDEN = [("la imagen primero (`imagen.jar:capa.jar`)", "gana la IMAGEN"),
         ("la capa primero (`capa.jar:imagen.jar`)", "gana la CAPA")]
# `provided` + `includeScope=runtime`: qué se copia de verdad.
PROVIDED = {
    "declarado por el repositorio": "jackson-dataformat-yaml 2.19.0",
    "provisto por la imagen": "jackson-core, jackson-databind, jackson-annotations 2.18.2, slf4j-api 2.0.16",
    "copiado a la capa": ["jackson-dataformat-yaml-2.19.0.jar", "snakeyaml-2.4.jar"],
}
# Y el caso feo: el repositorio pide una version de lo que la imagen ya pone.
CHOQUE = {
    "declarado": "jackson-databind 2.19.0 (directo) + commons-lang3 3.17.0",
    "la imagen pone": "jackson-databind 2.18.2, como `provided`, DESPUES en el pom",
    "maven dice": "[WARNING] 'dependencies.dependency...' must be unique: "
                  "com.fasterxml.jackson.core:jackson-databind:jar -> version 2.19.0 vs 2.18.2",
    "copiado": ["commons-lang3-3.17.0.jar"],
}
TIEMPOS = {"frio_s": 46.5, "caliente_s": 6.0, "offline_s": 5.7, "repo_local_mb": 24}


def titulo(t):
    print("\n" + t)
    print("  " + "-" * (len(t) - 2))


def leer(*p):
    return open(os.path.join(*p), encoding="utf-8", errors="replace").read()


def seccion_hoy():
    titulo("  §1 LO QUE LA CAPA ENTIENDE HOY")
    e = leer(RAIZ, "crates", "ore-serve", "src", "entorno.rs")
    ficheros = sorted(set(re.findall(r'"(pyproject\.toml|pom\.xml|build\.gradle[^"]*)"', e)))
    print("     `entorno.rs` lee: %s" % ", ".join(ficheros))
    print("     y de ahí, SÓLO `[project].dependencies` — lo demás del fichero se ignora")
    y = leer(RAIZ, "malla", "52-la-capa.yaml")
    print("     el Job de la capa resuelve con: %s"
          % ("`pip download`, ruedas para cp312/manylinux" if "pip download" in y else "?"))
    print("     y lo deja en: el bucket del inquilino (`ore/puesto/<digest>/`), con su")
    print("     informe en el árbol (`entorno/<digest>.json`)")
    p = leer(RAIZ, "malla", "51-el-puesto.yaml")
    print("     el puesto la baja en un init y la monta en `/capa`: %s"
          % ("PYTHONPATH=/capa" if "PYTHONPATH" in p and "/capa" in p else "?"))
    print("     ⇒ De punta a punta, la capa es de PYTHON. Un repositorio de Java no")
    print("       tiene dónde declarar, y por tanto NO PUEDE USAR NI UNA BIBLIOTECA.")
    print("       Lo dice la propia semilla desde 0036 ⑧b: «la capa lee `pyproject.toml`")
    print("       y hoy no sabe de la JVM».")


def seccion_por_que_no_vale():
    titulo("  §2 POR QUE LO DE PYTHON NO VALE TAL CUAL")
    print("     `pip download` baja RUEDAS para un intérprete concreto (cp312, manylinux)")
    print("     y el puesto las instala. En la JVM no hay ruedas ni intérprete: hay JARS,")
    print("     que valen para cualquier JDK ≥ al que los compiló, y un CLASSPATH.")
    print("     ⇒ La mecánica se parece —declarar, resolver fuera, subir, montar— pero")
    print("       quien resuelve y qué se monta son otros. Y una ventaja: un jar no")
    print("       depende de la versión del intérprete, así que la capa de la JVM no")
    print("       tiene el problema de ABI que la de Python tiene escrito en el Job.")


def seccion_quien_resuelve():
    titulo("  §3 QUIEN RESUELVE (medido)")
    for n, d in RESOLUTORES.items():
        print("     %-34s %6.2f MB de descarga · pide %s" % (n, d["mb"], d["pide"]))
    print("")
    print("     Maven, resolviendo DE VERDAD con un repositorio local vacío —lo que el")
    print("     Job tendría la primera vez— sobre `%s`:" % MAVEN["dependencia"])
    print("       %d jars · %.1f MB · %.0f s" % (MAVEN["jars"], MAVEN["mb"], MAVEN["segundos"]))
    print("     (la capa de Python, para comparar: %s)" % PYTHON)
    print("     ⛔ coursier no se pudo ejercitar aquí: su lanzador se rompe en esta")
    print("       máquina con `NoSuchMethodError` de cats. Es un problema del sitio, no")
    print("       un veredicto — pero lo que NO está medido no se cuenta, y por eso la")
    print("       propuesta va con Maven, que sí se ejercitó.")


def seccion_donde():
    titulo("  §4 DONDE CORRERIA")
    d = leer(RAIZ, "Dockerfile")
    tiene_jdk = "eclipse-temurin:21-jdk" in d
    print("     el Job de la capa de Python corre sobre `ore-drivers` (alpine + gcloud),")
    print("     que NO tiene Java.")
    print("     pero `puesto-jvm:1` sale de `eclipse-temurin:21-jdk-noble`: %s"
          % ("ya tiene el JDK" if tiene_jdk else "?"))
    print("     ⇒ El Job de la capa de la JVM puede correr sobre NUESTRA PROPIA imagen")
    print("       de puesto: el JDK ya está, y Maven son 9 MB que se bajan en el Job o")
    print("       se hornean una vez. Ni una imagen nueva.")
    print("     ⛔ Y la red: el Job de la capa la tiene (por eso resuelve); EL PUESTO NO,")
    print("       y eso no cambia. Sigue siendo «alguien con red resuelve, el pod sin")
    print("       red monta».")


def seccion_forma():
    titulo("  §5 LA FORMA QUE SALE")
    print("     · el repositorio declara en `pom.xml`, y se lee SÓLO `<dependencies>`.")
    print("       Es la misma regla que en Python —de `pyproject.toml` sólo se lee")
    print("       `[project].dependencies`— y por la misma razón: el fichero es el que")
    print("       la gente de ese lenguaje espera, y lo que no honramos no se finge.")
    print("       Inventarnos un formato nuestro sería el error de siempre, al revés.")
    print("     · el digest y el alcance, IGUAL que hoy (la raíz, su paquete, él).")
    print("     · el Job: `mvn dependency:copy-dependencies` a un directorio, subir al")
    print("       bucket, informe en el árbol. La misma forma, otro verbo.")
    print("     · el puesto: un init que baja los jars a `/capa`, y el agente arranca con")
    print("       `/capa/*` en el classpath.")
    print("     ⭐ Y EL SERVIDOR DE LENGUAJE SE ENTERA SOLO: `Lenguaje.java` toma el")
    print("       classpath de `java.class.path` del propio proceso (0037 ③b), así que")
    print("       lo que la capa monte se autocompleta y se comprueba sin una línea más.")


def seccion_choque():
    titulo("  §6 EL CHOQUE: la imagen YA lleva 14 jars")
    jars = [l.split() for l in leer(RAIZ, "puesto", "jvm", "jars.txt").splitlines()
            if l.strip() and not l.startswith("#")]
    print("     en `/opt/ore/lib` viajan %d jars (y `duckdb_jdbc`):" % len(jars))
    print("       %s" % ", ".join("%s %s" % (g.split("/")[-1], v) for g, v in jars[:6]))
    print("       %s" % ", ".join("%s %s" % (g.split("/")[-1], v) for g, v in jars[6:]))
    print("     ⇒ Si la capa trae OTRO `jackson-databind`, hay dos en el classpath. Y")
    print("       quien gana lo decide EL ORDEN, no la versión. Medido con dos jars que")
    print("       declaran la misma clase:")
    for que, quien in ORDEN:
        print("       · %-42s %s" % (que, quien))
    print("     ⛔ Y una trampa de Java que conviene saber: una constante")
    print("       `static final String` se INCRUSTA en quien la usa al compilar, así que")
    print("       ésa no la cambia ningún classpath. Se midió sin querer y por eso se")
    print("       dice: la prueba hay que hacerla con un método, no con una constante.")
    print("     ⚠️ Y EN PYTHON YA PASA, con el orden contrario: `PYTHONPATH=/capa` va")
    print("       ANTES que todo —medido: un `json` puesto en la capa tapa hasta el de la")
    print("       biblioteca estándar—. Es decir, hoy la capa de Python PUEDE tapar")
    print("       pandas, pyarrow o duckdb de la imagen. No es asunto de ③c arreglarlo,")
    print("       pero sí decirlo: en la JVM elegimos el orden contrario A PROPÓSITO.")


def seccion_como_se_evita():
    titulo("  §7 COMO SE EVITA (medido con Maven de verdad)")
    print("     La pieza es `provided`, que en Maven quiere decir exactamente esto: «el")
    print("     contenedor ya lo pone». Se genera el pom con LO QUE EL REPOSITORIO")
    print("     DECLARA y, detrás, los 14 de la imagen como `provided`; y se copia sólo")
    print("     el ámbito `runtime`.")
    print("")
    print("     caso 1 · el repositorio trae algo que ARRASTRA lo de la imagen")
    print("       declarado:  %s" % PROVIDED["declarado por el repositorio"])
    print("       provisto:   %s" % PROVIDED["provisto por la imagen"])
    print("       COPIADO:    %s" % ", ".join(PROVIDED["copiado a la capa"]))
    print("       ⇒ Ni un jackson en la capa: el de la imagen manda y no hay duplicado.")
    print("")
    print("     caso 2 · el repositorio PIDE otra versión de lo que la imagen pone")
    print("       declarado:  %s" % CHOQUE["declarado"])
    print("       la imagen:  %s" % CHOQUE["la imagen pone"])
    print("       maven:      %s" % CHOQUE["maven dice"][:88])
    print("       COPIADO:    %s" % ", ".join(CHOQUE["copiado"]))
    print("       ⇒ GANA EL CONTENEDOR, y se entera todo el mundo: Maven lo grita en el")
    print("         registro del Job. Esa advertencia es la que el informe tiene que")
    print("         convertir en una frase que la consola pueda enseñar.")


def seccion_repetible():
    titulo("  §8 REPETIBLE Y SIN RED")
    print("     resolver en frío (repositorio local vacío): %.1f s" % TIEMPOS["frio_s"])
    print("     otra vez, con el repositorio local poblado: %.1f s" % TIEMPOS["caliente_s"])
    print("     y OFFLINE (`mvn -o`), sin tocar la red:      %.1f s ✓" % TIEMPOS["offline_s"])
    print("     el repositorio local, para ese caso:          %d MB" % TIEMPOS["repo_local_mb"])
    print("     ⭐ Que `-o` funcione no es un detalle: es la prueba de que la resolución")
    print("       es REPETIBLE. Y `-C` (política estricta de sumas) estuvo puesto en")
    print("       todas las medidas: un jar que no cuadre con su `.sha1` de Central")
    print("       rompe la resolución en vez de colarse.")
    print("     ⛔ Lo que NO es repetible por sí solo es el CONJUNTO resuelto: Maven")
    print("       vuelve a mediar versiones cada vez, y un `2.19.0` de hoy puede ser otro")
    print("       árbol mañana si alguien declara rangos. Por eso el informe tiene que")
    print("       llevar el CONJUNTO EXACTO —`dependency:list`, un GAV por línea— igual")
    print("       que la capa de Python lleva su `requirements.lock`.")


def seccion_bordes():
    titulo("  §9 LOS BORDES, dichos antes de construir")
    print("     · EL TAMAÑO. Una dependencia de Java puede ser enorme (un Spark son")
    print("       cientos de MB) y hoy la capa de Python no tiene tope escrito. Aquí")
    print("       hace falta uno, con su mensaje: más vale un «no cabe» que un puesto")
    print("       que tarda dos minutos en arrancar.")
    print("     · LA VERSION DEL JDK. Un jar compilado para 25 no corre en nuestro 21:")
    print("       es un error legible en cuanto se usa, pero el informe debería decirlo")
    print("       al resolver y no al primer `Run`.")
    print("     · EL ALCANCE. Tres poms (raíz, paquete, repositorio) se unen como los")
    print("       tres `pyproject.toml`: una lista de dependencias no tiene orden, así")
    print("       que la unión ordenada y sin repetidos vale igual y el digest también.")
    print("     · LOS PLUGINS. Un `pom.xml` puede traer `<build>`, `<plugins>` y")
    print("       ejecuciones: NO SE HONRAN Y NO SE FINGEN, exactamente como en Python")
    print("       no se honra nada de `pyproject.toml` fuera de `[project].dependencies`.")
    print("       Y el plugin que SÍ corre —`maven-dependency-plugin`— va con versión")
    print("       fija: si no, el Job ejecuta el código que Central tenga ese día.")


def main():
    print("=== 0037 ③c · la capa de la JVM, medida")
    seccion_hoy()
    seccion_por_que_no_vale()
    seccion_quien_resuelve()
    seccion_donde()
    seccion_forma()
    seccion_choque()
    seccion_como_se_evita()
    seccion_repetible()
    seccion_bordes()
    return 0


if __name__ == "__main__":
    sys.exit(main())
