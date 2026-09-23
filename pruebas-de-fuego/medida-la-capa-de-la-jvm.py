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


def main():
    print("=== 0037 ③c · la capa de la JVM, medida")
    seccion_hoy()
    seccion_por_que_no_vale()
    seccion_quien_resuelve()
    seccion_donde()
    seccion_forma()
    return 0


if __name__ == "__main__":
    sys.exit(main())
