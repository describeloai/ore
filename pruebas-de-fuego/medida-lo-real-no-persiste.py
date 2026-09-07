# -*- coding: utf-8 -*-
"""Por que no queda nada de lo real, y que costaria que quedara.

Se descubrieron 10 tablas contra BigQuery `trino-k8s`, se les anadio una vista
a mano, y hoy no hay ni rastro. La pregunta no es donde se fue el directorio:
es si el arbol tiene SITIO para lo que sale de un origen de verdad, y que deja
de estar cubierto porque no lo tiene.

  A. POR QUE NO QUEDO NADA     y que del arbol lo impedia (nada)
  B. DE QUE SE ALIMENTA        1777 ficheros, y cuantos vienen de un origen
  C. DONDE EMPIEZA LA SUITE    el eslabon que ninguna prueba cruza
  D. QUE NO EJERCE NADIE       medido por forma, no por impresion
  E. QUE PUEDE VIAJAR          el precedente `.oretopo`, y por que no aplica
  F. LO QUE COSTARIA           el artefacto mas barato que cierra el hueco
"""
import pathlib
import subprocess
import textwrap

RAIZ = pathlib.Path(r"C:\ORE")


def parrafo(t, sangria="     ", ancho=68):
    for l in textwrap.wrap(t, ancho):
        print("%s%s" % (sangria, l))


def contar(patron, *dirs, ext="*.yaml"):
    n = 0
    for d in dirs:
        for f in (RAIZ / d).rglob(ext):
            try:
                if patron in f.read_text(encoding="utf-8", errors="replace"):
                    n += 1
            except OSError:
                pass
    return n


print("== lo real no persiste, medido ==")

# -- A -----------------------------------------------------------------------
print()
print("A - POR QUE NO QUEDO NADA")
print()
gitignore = (RAIZ / ".gitignore").read_text(encoding="utf-8")
log = subprocess.run(["git", "log", "--oneline", "--", "packages"],
                     cwd=RAIZ, capture_output=True, text=True).stdout.strip()
print("   `packages/` en .gitignore        %s" % ("SI" if "packages" in gitignore else "no"))
print("   `packages/` en el historial      %s" % ("si" if log else "NUNCA"))
print("   el directorio, en el arbol       %s" % ("si" if (RAIZ / "packages").exists() else "no existe"))
print()
parrafo("Asi que no lo impidio ninguna regla: se escribio FUERA del arbol y el "
        "arbol no tenia donde ponerlo. Lo unico que el repositorio prohibe "
        "explicitamente es `*.oretopo` —ver (E)—, y un paquete descubierto no "
        "es eso.")

# -- B -----------------------------------------------------------------------
print()
print("B - DE QUE SE ALIMENTA LA SUITE HOY")
print()
yamls = sum(1 for _ in (RAIZ / "vendor/oos/examples").rglob("*.yaml")) + \
        sum(1 for _ in (RAIZ / "vendor/oos/conformance").rglob("*.yaml"))
reales = contar("bigquery://", "vendor/oos/examples", "vendor/oos/conformance")
incrustados = 0
ficheros_con = 0
for f in (RAIZ / "crates").rglob("*.rs"):
    t = f.read_text(encoding="utf-8", errors="replace")
    c = t.count("apiVersion: oos.dev")
    if c:
        ficheros_con += 1
        incrustados += c
print("   ficheros YAML de fixture              %4d" % yamls)
print("   ...que nombran un origen real         %4d" % reales)
print("   documentos OOS incrustados en .rs     %4d  (en %d ficheros)" % (incrustados, ficheros_con))
print()
parrafo("Casi dos mil fixtures y CERO vienen de un origen. Los datasources se "
        "llaman `erp_snowflake`, `hr_workday`, `acme.example`: los escribimos "
        "nosotros, con la forma que creemos que tienen. Eso no es un defecto "
        "de las pruebas —hacen falta y son exactas—, es que TODAS son del "
        "mismo tipo.")

# -- C -----------------------------------------------------------------------
print()
print("C - DONDE EMPIEZA LA SUITE, Y EL ESLABON QUE NO CRUZA")
print()
d = (RAIZ / "crates/ore-cli/tests/descubrimiento.rs").read_text(encoding="utf-8")
print("   `descubrimiento.rs` lo dice de si mismo:")
print()
print("     «esto empieza donde acaba el driver, con un catalogo escrito a mano»")
print()
parrafo("Y esta bien razonado: lo que necesita servidor es CONSEGUIR el "
        "catalogo, y lo que pasa despues no. Pero de ahi sale una consecuencia "
        "que no estaba escrita: lo unico que ninguna prueba comprueba es si el "
        "catalogo que escribimos a mano SE PARECE al que emite un driver. Y esa "
        "es justo la costura donde se han caido las cosas esta semana.")
print()
print("   Lo que se ha encontrado ejecutando, y no razonando:")
for q in [
    "`.ndjson` como nombre de objeto — el inductor colisionaba porque el punto "
    "es separador en el contrato del catalogo",
    "`ore source check` mandaba la URL pelada y `check` espera la forma de "
    "`leer_coordenada`",
    "`bq` «no arranca en esta maquina» — arrancaba, y desde Git Bash no",
]:
    print()
    parrafo("· " + q, "     ")

# -- D -----------------------------------------------------------------------
print()
print("D - QUE FORMA NO EJERCE NINGUN FIXTURE")
print()
FORMAS = [
    "projectionPushdown", "requiredFilters", "fullScan: forbidden",
    "fullScan: expensive", "witness: snapshot", "witness: field",
]
print("   %-24s %s" % ("forma", "ficheros que la llevan"))
print("   " + "-" * 60)
for k in FORMAS:
    n = contar(k, "vendor/oos/examples", "vendor/oos/conformance")
    print("   %-24s %s" % (k, n if n else "NINGUNO"))
print()
parrafo("`projectionPushdown` es la palabra que se anadio esta semana, con "
        "`OOS2029` detras, y NO HAY UN SOLO FICHERO que la lleve: se ejerce solo "
        "desde YAML incrustado en una prueba. Y las tres que el catalogo de "
        "BigQuery deriva de verdad —particion obligatoria a `fullScan: "
        "forbidden` mas `requiredFilters`, y el resto a `expensive`— se apoyan "
        "en dos ficheros escritos por nosotros.")

# -- E -----------------------------------------------------------------------
print()
print("E - QUE PUEDE VIAJAR Y QUE NO")
print()
print("   El arbol ya tiene una regla sobre esto, y es la unica:")
print()
parrafo("«El artefacto de topologia CONTIENE DATOS —saber que el paciente X "
        "esta enlazado con la clinica Y es el diagnostico (ADR 0006 §4)—, asi "
        "que no viaja con la configuracion.»  -> `*.oretopo` en .gitignore")
print()
print("   Y el criterio se aplica solo:")
print()
print("   %-26s %-10s %s" % ("artefacto", "¿lleva datos?", "¿puede viajar?"))
print("   " + "-" * 66)
for a, datos, viaja in [
    ("`.oretopo`", "SI, filas", "no, y lo cobra una prueba"),
    ("catalogo de un driver", "no: nombres y tipos", "SI"),
    ("paquete descubierto", "no: declaraciones", "SI"),
    ("una copia materializada", "SI, filas", "no"),
]:
    print("   %-26s %-14s %s" % (a, datos, viaja))
print()
parrafo("Un catalogo dice que columnas hay y de que tipo, y que sabe hacer el "
        "origen. No lleva una sola fila. La razon por la que la topologia no "
        "viaja NO le aplica.")

# -- F -----------------------------------------------------------------------
print()
print("F - EL ARTEFACTO MAS BARATO QUE CIERRA EL HUECO")
print()
parrafo("NO es el paquete descubierto: es el CATALOGO que emitio el driver. "
        "Un fichero, en la frontera exacta donde la suite hoy empieza a "
        "suponer, y con la forma que `descubrimiento.rs` YA consume —solo que "
        "escrita a mano en vez de capturada—.")
print()
print("   lo que hace falta            un `.json` por origen real, capturado")
print("   donde                        junto a la prueba que ya lo consume")
print("   que ejerce                   induccion, `validate`, `view`, la cola")
print("   que NO exige                 servidor: se captura una vez y se lee")
print("   que cuesta mantener          se recaptura cuando el origen cambie,")
print("                                y ESE diff es la deteccion de deriva")
print()
parrafo("Y el segundo, si el primero vale: el mismo catalogo por cada familia "
        "—BigQuery, PostgreSQL, jsonl—, que es lo que convierte «los tres "
        "drivers emiten la misma forma» de intencion en aserto. Hoy esa frase "
        "esta escrita en tres cabeceras y no la comprueba nadie.")
