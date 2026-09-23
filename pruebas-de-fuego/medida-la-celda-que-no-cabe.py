"""0031 W3 · LA CELDA QUE NO CABE, MEDIDA.

Una sesión corre en UNA JVM dentro de UN pod. La pregunta que decide el techo
del modelo no es «¿va rápido?» sino **«¿qué pasa el día que el dato no cabe?»**
— y hay dos respuestas posibles y muy distintas: que se cuente (una celda con
error, la sesión viva) o que mate (el pod fuera, el trabajo perdido).

  §1  CUANTO HAY DE VERDAD     lo que un puesto da, lo que el inquilino tiene,
                               y lo que la JVM se queda de eso.
  §2  CUANTO CUESTA UNA FILA   bytes por fila materializada, y qué significa de
                               verdad el límite de 1 M.
  §3  QUE PASA CUANDO NO CABE  medido: tres finales, y sólo uno es bueno.
  §4  LA SALIDA QUE YA EXISTE  DuckDB fuera del núcleo, medido con 12 M de
                               grupos en 200 MB.
  §5  LO QUE CAMBIA AL SUBIR   compartido · dedicado · BYOC (0024 ①), y qué
                               significa cada uno para «no cabe».
  §6  Y SPARK                  qué compraría, qué costaría, y qué pregunta hay
                               que contestar antes.

    python pruebas-de-fuego/medida-la-celda-que-no-cabe.py
"""
import os
import re
import sys

sys.stdout.reconfigure(encoding="utf-8", errors="replace")
AQUI = os.path.dirname(os.path.abspath(__file__))
RAIZ = os.path.dirname(AQUI)

# ─────────────────────────────────────────────────────────────────────────────
# Medido el 2026-09-23 con el JDK 21 de Temurin (el mismo que `puesto-jvm:1`),
# los jars de `puesto/jvm/jars.txt` y el `duckdb_jdbc` de la imagen. El tamaño
# del contenedor se simula con `-XX:MaxRAM=4g`, que es lo que la JVM ve dentro
# de un pod con `limits.memory: 4Gi`.
# ─────────────────────────────────────────────────────────────────────────────

# §1 · lo que la JVM se queda de un contenedor de 4 GiB.
HEAP_MB = 989          # `java -XX:MaxRAM=4g -XshowSettings:vm`: el 25% por defecto
DIRECTA = "0"          # `MaxDirectMemorySize` sin fijar = lo mismo que el heap

# §2 · bytes por fila materializada (`List<Map<String,Object>>`, la forma exacta
#      que devuelven `over()` y `sql()`), con los tipos del contrato (0032).
FILA = {4: 639, 12: 1585, 24: 3188}

# §3 · los tres finales.
OOM_CELDA = {"vuelve": "EvalException: Java heap space", "ms": 442,
             "sesion": "SIGUE VIVA (2+2=4, y la variable de antes sigue ahí)",
             "retenido_mb": 232, "heap_mb": 256}
ARROW = {"limite": "Long.MAX_VALUE (SIN LIMITE)", "reservado_mb": 200,
         "con_tope_directo_mb": 64, "heap_mb": 989}

# §4 · DuckDB fuera del núcleo.
DUCK = {"por_defecto_memory_limit": "25.0 GiB (en una máquina de 32 GB)",
        "por_defecto_temp_directory": ".tmp",
        "sin_derrame": "Out of Memory Error: could not allocate block of size 256.0 KiB "
                       "(190.5 MiB/190.7 MiB used) … Unused blocks cannot be offloaded to disk",
        "con_derrame_s": 12.15, "grupos": 12_000_000, "tope": "200MB"}


def titulo(t):
    print("\n" + t)
    print("  " + "-" * (len(t) - 2))


def leer(*p):
    return open(os.path.join(*p), encoding="utf-8", errors="replace").read()


def seccion_cuanto_hay():
    titulo("  §1 CUANTO HAY DE VERDAD")
    p = leer(RAIZ, "malla", "51-el-puesto.yaml")
    pide = re.findall(r'requests: \{cpu: "([^"]+)", memory: (\S+)\}', p)
    tope = re.findall(r'limits:\s+\{cpu: "([^"]+)", memory: (\S+)\}', p)
    print("     el pod del puesto (`51-el-puesto.yaml`):")
    if pide and tope:
        print("       pide %s CPU / %s   ·   tope %s CPU / %s"
              % (pide[-1][0], pide[-1][1], tope[-1][0], tope[-1][1].rstrip(",}")))
    q = leer(RAIZ, "malla", "11-el-inquilino.yaml")
    cpu = re.search(r'name: cpu\s+nominalQuota: "(\d+)"', q)
    mem = re.search(r'name: memory\s+nominalQuota: "(\S+?)"', q)
    if cpu and mem:
        print("     el inquilino entero (cola de Kueue): %s CPU · %s para TODOS sus Jobs"
              % (cpu.group(1), mem.group(1)))
    print("     el clúster compartido (0024): 3 nodos e2-standard-4 spot = 12 vCPU · 48 GiB")
    print("     ⇒ el pod más grande que cabe es UN NODO: ~13 GiB útiles. Ese es el techo")
    print("       vertical de hoy, y no lo sube ninguna decisión de código.")
    print("")
    print("     ⛔ Y DE ESOS 4 GiB, LA JVM SE QUEDA CON EL 25%%: %d MB de heap." % HEAP_MB)
    print("       No es un ajuste nuestro: es el valor por defecto de la JVM en un")
    print("       contenedor (`MaxRAMPercentage=25`), y el `CMD` de `puesto-jvm:1` no")
    print("       lo toca. Medido con `-XX:MaxRAM=4g`.")
    print("       ⇒ 3 de cada 4 GiB del pod NO son para los objetos de la celda.")
    print("       Sirven para el resto (Arrow fuera del heap, DuckDB, el propio JDK),")
    print("       pero nadie lo decidió: salió así.")


def seccion_la_fila():
    titulo("  §2 CUANTO CUESTA UNA FILA, Y QUE PROTEGE DE VERDAD EL LIMITE")
    o = leer(RAIZ, "puesto", "jvm", "ore", "Ore.java")
    m = re.search(r"LIMITE = ([\d_]+)", o)
    limite = int(m.group(1).replace("_", "")) if m else 1_000_000
    print("     `Ore.LIMITE` = %s filas: lo que `over()` y `sql()` materializan" % f"{limite:,}".replace(",", "."))
    print("     como `List<Map<String,Object>>` antes de truncar.")
    print("")
    print("     Lo que cuesta una fila de VERDAD (medido, con los tipos del contrato):")
    for cols, b in FILA.items():
        mb = b * limite / 1048576
        cabe = "cabe" if mb < HEAP_MB * 0.6 else ("JUSTO" if mb < HEAP_MB else "NO CABE")
        print("       %2d columnas → %5d bytes/fila → %s filas serían %6.0f MB   %s"
              % (cols, b, "1 M", mb, cabe))
    print("")
    print("     ⭐ EL LIMITE ESTA EN FILAS Y LO QUE SE AGOTA SON BYTES. Con 4 columnas")
    print("       el millón entra (a costa del 60% del heap); con 12 pide 1,5 GB y con")
    print("       24 pide 3 GB — o sea que en un dataset normal la sesión se queda sin")
    print("       memoria ANTES de llegar al límite que existe para protegerla.")
    print("       El límite protege del tamaño de la RESPUESTA, no de la memoria.")


def seccion_no_cabe():
    titulo("  §3 QUE PASA CUANDO NO CABE: tres finales, y sólo uno es bueno")
    print("     ① EL HEAP SE ACABA DENTRO DE LA CELDA  ·  medido, y SALE BIEN")
    print("       JShell con `executionEngine(\"local\")` —el del agente— evaluando algo")
    print("       que se come el heap devuelve  %s" % OOM_CELDA["vuelve"])
    print("       en %d ms, como una celda con error cualquiera. Y LA SESION %s"
          % (OOM_CELDA["ms"], OOM_CELDA["sesion"]))
    print("       ⇒ Se pierde la celda, no el trabajo. Que es lo que hay que querer.")
    print("       ⚠️ Con una pega medida: lo que la celda muerta alcanzó a reservar")
    print("         SIGUE RESERVADO —%d MB de %d— porque la variable vive en la"
          % (OOM_CELDA["retenido_mb"], OOM_CELDA["heap_mb"]))
    print("         sesión. La siguiente celda grande muere antes. La sesión sobrevive")
    print("         envenenada hasta que alguien suelte la variable.")
    print("")
    print("     ② LA MEMORIA SE PIDE FUERA DEL HEAP  ·  medido, y NO LA PARA NADIE")
    print("       El asignador de Arrow (`new RootAllocator()`) tiene de tope %s." % ARROW["limite"])
    print("       Y no se le aplica el tope de memoria directa de la JVM: con")
    print("       `-XX:MaxDirectMemorySize=%dm` reservó %d MB SIN ERROR (heap %d MB),"
          % (ARROW["con_tope_directo_mb"], ARROW["reservado_mb"], ARROW["heap_mb"]))
    print("       porque `arrow-memory-unsafe` pide al sistema operativo, no a la JVM.")
    print("       ⇒ El camino RAPIDO —`arrow()`, `arrowSql()`, y lo que usa `over()`")
    print("         por debajo— no tiene freno en la JVM. El único que queda es el")
    print("         tope del contenedor, y ése no lanza una excepción: MATA EL POD")
    print("         (OOMKilled). Con `restartPolicy: Never`, el puesto se pierde.")
    print("       ⛔ Es la asimetría que hay que arreglar: el camino seguro falla")
    print("         contándolo y el camino rápido falla matando.")
    print("       ⇒ Se arregla en una línea: `new RootAllocator(tope)` con un tope")
    print("         derivado del contenedor. Entonces ② se convierte en ①.")
    print("")
    print("     ③ EL PROCESO SE PASA DEL TOPE DEL POD  ·  no medido aquí, y se sabe")
    print("       El kernel manda SIGKILL; no hay excepción, no hay mensaje, no hay")
    print("       informe. En Python es el final POR DEFECTO —CPython no tiene tope de")
    print("       heap: el que para es el kernel—, así que un `pandas` grande mata el")
    print("       puesto donde la JVM habría devuelto un error. Java sale ganando aquí,")
    print("       y no por diseño nuestro.")


def seccion_la_salida():
    titulo("  §4 LA SALIDA QUE YA EXISTE: DuckDB sabe no caber")
    print("     DuckDB trabaja FUERA DEL NUCLEO: si no cabe, derrama a disco. Medido")
    print("     con el `duckdb_jdbc` de la imagen, agrupando %s claves distintas:"
          % f"{DUCK['grupos']:,}".replace(",", "."))
    print("       memory_limit='%s' y SIN sitio donde derramar:" % DUCK["tope"])
    print("         %s" % DUCK["sin_derrame"][:96])
    print("       memory_limit='%s' y CON `temp_directory`:" % DUCK["tope"])
    print("         RESULTADO OK en %.1f s" % DUCK["con_derrame_s"])
    print("     ⇒ «no cabe» puede significar «tarda 12 s» en vez de «no se puede», y")
    print("       la pieza que lo decide son dos `set`.")
    print("")
    print("     ⛔ Y LO QUE HAY HOY NO ESTA PUESTO. Medido: la conexión se abre con")
    print("       `DriverManager.getConnection(\"jdbc:duckdb:\")` (y `duckdb.connect()`")
    print("       en Python) SIN memory_limit y SIN temp_directory. Por defecto DuckDB")
    print("       se pone %s — o sea, LO SACA DE LA MAQUINA," % DUCK["por_defecto_memory_limit"])
    print("       no del pod. En un contenedor de 4 GiB eso es pedirle al kernel que")
    print("       mate el puesto.")
    print("     ⚠️ LO QUE FALTA MEDIR, y es una celda: dentro de `puesto-jvm:1` de")
    print("       verdad, `select current_setting('memory_limit')`. Si DuckDB lee el")
    print("       límite del cgroup, el riesgo es menor; si lee la máquina, es el")
    print("       primer arreglo de todos. No se supone: se mira.")


def seccion_al_subir():
    titulo("  §5 LO QUE CAMBIA AL SUBIR DE TIER (0024 ①)")
    print("     compartido  nuestro clúster, namespace `t-<n>`, cola de Kueue.")
    print("                 El techo es la cuota (36 GiB para TODO el inquilino) y el")
    print("                 nodo (13 GiB el pod más grande). No se mueve por código.")
    print("     dedicado    un clúster por inquilino: los node pools son SUYOS, así que")
    print("                 «un puesto de 64 GiB» pasa a ser una decisión de pool y no")
    print("                 una pelea de cuota. El techo vertical sube de golpe.")
    print("     BYOC        la cuenta del cliente: el techo es su presupuesto, y el")
    print("                 dato YA ESTA AHI —es el caso donde traer el cómputo al dato")
    print("                 deja de ser una frase y es la topología—.")
    print("")
    print("     ⭐ LO QUE ESTO SIGNIFICA PARA «NO CABE», y es lo importante:")
    print("       En compartido, distribuir es caro y el techo es bajo: la respuesta")
    print("       sensata es NO MATERIALIZAR (agregar en SQL, `arrow()` por lotes).")
    print("       En dedicado y BYOC, el techo vertical es tan alto que UNA MAQUINA")
    print("       GRANDE resuelve casi todo lo que hoy llamaríamos «hace falta un")
    print("       clúster»: 1 TB de RAM en una sola máquina es una compra, no un")
    print("       proyecto. Distribuir deja de ser la primera respuesta y pasa a ser")
    print("       la última.")


def seccion_spark():
    titulo("  §6 Y SPARK: qué compraría, qué costaría")
    print("     LO QUE COMPRA (y no es poco):")
    print("       · pasar de «lo que cabe en un pod» a «lo que cabe en N pods»;")
    print("       · derrame, reintentos y barajado ya resueltos;")
    print("       · un vocabulario que la gente de datos ya conoce.")
    print("")
    print("     LO QUE CUESTA, con los números de esta misma medida delante:")
    print("       · UN DRIVER Y N EJECUTORES, no un pod: el suelo de una sesión pasa")
    print("         de 4 GiB a varios GiB SIEMPRE, también para el `select * limit 10`")
    print("         que es el 90% de lo que se escribe;")
    print("       · EL CODIGO DE LA CELDA TIENE QUE VIAJAR. Lo que una celda define")
    print("         son clases que sólo existen en el driver; Spark las sirve a los")
    print("         ejecutores por un servidor de clases. Nuestro agente NO tiene ese")
    print("         problema hoy porque corre en una JVM: el día que distribuya, lo")
    print("         tiene entero, y es el problema más caro del cambio;")
    print("       · DOS MOTORES O UNO. Hoy `sql()` y `over()` son DuckDB en los tres")
    print("         entornos, y el contrato de tipos (0032) está medido contra él.")
    print("         Meter Spark debajo de TODO es cambiar el motor de la verdad —y")
    print("         volver a medir los 23 tipos—; meterlo AL LADO es tener dos")
    print("         semánticas de `null`, de decimal y de zona horaria.")
    print("")
    print("     ⇒ LA PREGUNTA QUE HAY QUE CONTESTAR ANTES, y no es sobre Spark:")
    print("       ¿cuántos de los datasets reales de un cliente NO caben en una")
    print("       máquina grande, agregando en SQL? Si la respuesta es «casi ninguno»,")
    print("       Spark es un motor de tercera parte que se paga en todas las celdas")
    print("       para servir a unas pocas. Si es «muchos», el sitio de Spark es el")
    print("       TRABAJO (0031 §9) —que ya corre como Job y no tiene a nadie")
    print("       delante—, no la sesión: la celda sigue siendo para mirar, y lo")
    print("       masivo se declara y se manda.")


def main():
    print("=== 0031 W3 · la celda que no cabe, medida")
    print("    (una sesión = una JVM en un pod; esto dice dónde está el borde)")
    seccion_cuanto_hay()
    seccion_la_fila()
    seccion_no_cabe()
    seccion_la_salida()
    seccion_al_subir()
    seccion_spark()
    print("")
    print("  ⇒ EN UNA FRASE: el borde no es «la celda no cabe» —eso se cuenta y la")
    print("    sesión sobrevive—, es QUE HAY DOS CAMINOS QUE NO SE CUENTAN: el")
    print("    asignador de Arrow sin tope y DuckDB creyéndose el dueño de la")
    print("    máquina. Los dos se arreglan con un tope escrito, y los dos convierten")
    print("    un pod muerto en un error legible. Eso vale más, hoy, que distribuir.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
