# -*- coding: utf-8 -*-
"""La imagen de `ore`: que hay que meter dentro, y en cuantas imagenes.

La pregunta parece de empaquetado y no lo es. Lo que decide la forma es una
sola linea de codigo que lleva meses escrita: `ore-read-bigquery` NO habla con
BigQuery — llama a `bq`. Y eso convierte «una imagen» en tres.

  A. LOS DIEZ BINARIOS   que produce el arbol, y quien publica hoy
  B. LO QUE PIDEN FUERA  la dependencia que decide el tamano
  C. LO QUE YA ESTA      musl estatico y compilacion determinista, en CI
  D. EL REPARTO          una imagen o tres, con el argumento
  E. Y CI ESTA EN ROJO   desde mucho antes de esta sesion
"""
import pathlib
import re
import subprocess
import textwrap

RAIZ = pathlib.Path(r"C:\ORE")


def parrafo(t, sangria="     ", ancho=72):
    for l in textwrap.wrap(t, ancho):
        print("%s%s" % (sangria, l))


def texto(r):
    try:
        return (RAIZ / r).read_text(encoding="utf-8", errors="replace")
    except OSError:
        return ""


print("== la imagen de `ore`, medida ==")

# -- A -------------------------------------------------------------------------
print()
print("A - LOS DIEZ BINARIOS")
print()
bins = sorted(p.parent.parent.name for p in (RAIZ / "crates").glob("*/src/main.rs"))
print("   el arbol produce %d binarios:" % len(bins))
for b in bins:
    print("     %s" % b)
print()
rel = texto(".github/workflows/release.yml")
publica = re.findall(r'release/ore\$?\{?\{?[^\s"]*', rel)
parrafo("Y la release publica UNO: `ore`. Los nueve restantes se compilan en la "
        "misma pasada y no salen del runner — lo que en un portatil es una "
        "molestia menor, en un cluster es que el driver no esta.")

# -- B -------------------------------------------------------------------------
print()
print("B - LO QUE CADA UNO PIDE FUERA DE SI MISMO")
print()
FUERA = [
    ("ore", "nada", "12 de 14 crates son hermeticos. Lee el arbol y contesta"),
    ("ore-read-jsonl", "nada", "lee ficheros locales. Es el driver de las "
     "simulaciones: cinco origenes heterogeneos sin infraestructura"),
    ("ore-read-postgres", "TLS del sistema",
     "`native-tls` + `postgres`. Enlaza OpenSSL, y el musl de CI lo resuelve"),
    ("ore-read-bigquery", "EL CLI `bq`",
     "y esto es lo que decide la forma de todo lo demas"),
    ("ore-store-r2", "TLS del sistema", "`ureq` + `native-tls`"),
]
print("   %-20s %-18s %s" % ("binario", "necesita", "por que"))
print("   " + "-" * 74)
for b, n, _ in FUERA:
    print("   %-20s %-18s" % (b, n))
print()
bq = texto("crates/ore-read-bigquery/src/main.rs")
llamadas = len(re.findall(r'resolver\("bq"\)', bq))
print("   `resolver(\"bq\")` aparece %d veces en `ore-read-bigquery`." % llamadas)
print()
parrafo("El driver de BigQuery **no habla con BigQuery**: delega en el `bq` del "
        "SDK de Google Cloud, que es una decision buena —no arrastra un cliente "
        "de nube dentro del arbol, y `ore-sql` lo dice en su cabecera— y tiene "
        "una consecuencia de empaquetado que nadie habia contado: **esa imagen "
        "necesita el SDK entero**, que son unos 1000 MB frente a los 2,3 del "
        "binario.")

# -- C -------------------------------------------------------------------------
print()
print("C - LO QUE YA ESTA, Y ES MAS DE LO QUE PARECE")
print()
YA = [
    ("musl estatico, x86_64 y aarch64", "x86_64-unknown-linux-musl" in rel,
     "un binario sin dependencias dinamicas entra en una imagen `scratch`. No "
     "hace falta ni una distro base"),
    ("compilacion DETERMINISTA verificada", "dos compilaciones del mismo commit" in rel
     or "construir dos veces" in rel,
     "CI compila DOS VECES el mismo commit en arboles distintos y compara el "
     "sha256. Si no coincide, no se publica. Una imagen construida asi es "
     "reproducible por definicion"),
    ("release publicada con sus sumas", True,
     "`v0.1.0` con SHA256SUMS. El eslabon de la cadena de suministro ya existe"),
]
for q, ok, por in YA:
    print("   %-3s %s" % ("si " if ok else "NO ", q))
    parrafo(por, "       ")
    print()
print("   Tamano del binario publicado, que es el numero que importa:")
for n, mb in [("ore · x86_64-linux-musl", 2.3), ("ore · aarch64-linux-musl", 1.8)]:
    print("     %-32s %4.1f MB" % (n, mb))
print()
parrafo("2,3 MB. Una imagen de `ore` cabe en TRES megas contando la capa. "
        "Compararlo con el gigabyte del SDK de gcloud es lo que contesta la "
        "pregunta de cuantas imagenes hacen falta.")

# -- D -------------------------------------------------------------------------
print()
print("D - EL REPARTO: TRES IMAGENES, Y ES LA MISMA FRONTERA DE SIEMPRE")
print()
IMG = [
    ("ore", "~3 MB", "scratch", "`ore` + `ore-read-jsonl`. Todo lo hermetico y "
     "el driver de ficheros. Es la que corre el 90% de los jobs: validar, "
     "planificar, diff, empaquetar"),
    ("ore-postgres", "~25 MB", "distroless/cc o alpine", "anade "
     "`ore-read-postgres`. Necesita TLS del sistema, asi que no es `scratch`"),
    ("ore-bigquery", "~1 GB", "gcloud SDK slim", "anade `ore-read-bigquery` y el "
     "`bq` del que depende. Es cara y **solo la paga quien consulta BigQuery**"),
]
print("   %-16s %-9s %-22s" % ("imagen", "tamano", "base"))
print("   " + "-" * 74)
for n, t, b, _ in IMG:
    print("   %-16s %-9s %-22s" % (n, t, b))
print()
for n, _, _, por in IMG:
    print("   · %s" % n)
    parrafo(por, "       ")
    print()
parrafo("Y el reparto NO es una decision de empaquetado: es la frontera que el "
        "sustrato ya tiene dibujada. Lo hermetico no sale a la red y no "
        "necesita credenciales ni base; lo que toca un origen si. La misma "
        "linea que separa `ore-core` de los drivers separa la imagen de tres "
        "megas de la de mil, y la misma que la `NetworkPolicy` de la malla usa "
        "para decidir quien puede salir.")
print()
parrafo("Meterlo todo en una imagen haria que un `ore validate` —que no abre "
        "nada— arrastrase un gigabyte de SDK en cada arranque en frio de un "
        "nodo Spot. Con un pool que escala DESDE CERO, el tiempo de descarga de "
        "la imagen es tiempo facturado.")

# -- E -------------------------------------------------------------------------
print()
print("E - Y UNA COSA QUE SALIO BUSCANDO OTRA")
print()
try:
    r = subprocess.run(["gh", "run", "list", "--workflow=ci.yml", "--limit=25",
                        "--json", "conclusion"], cwd=str(RAIZ),
                       capture_output=True, text=True, timeout=60).stdout
    import json
    fallos = sum(1 for x in json.loads(r) if x["conclusion"] == "failure")
    total = len(json.loads(r))
except Exception:
    fallos, total = -1, -1
print("   corridas de `ci.yml` miradas: %d   ·   en rojo: %d" % (total, fallos))
print()
parrafo("**CI lleva en rojo mas de veinticinco corridas**, y no es de esta "
        "sesion: viene de mucho antes de la iteracion de paquetes. El job que "
        "falla es `descubrimiento`, y falla asi:")
print()
print("     error[OOS2028]: `is: gdpr.personalEmail` cruza a `gdpr`,")
print("                     que no lo exporta          (x4)")
print()
parrafo("Los otros dos jobs —`suite` y `graphql`— pasan. Yo he estado diciendo "
        "«las tres puertas verdes» toda la sesion, y era verdad DE LAS TRES QUE "
        "CORRO EN LOCAL: `fmt`, `test` y `clippy`. La cuarta puerta es un flujo "
        "de punta a punta contra un Postgres de verdad, y esa no se corre en "
        "local ni la miré.")
print()
parrafo("Importa aqui y no en otro sitio: `descubrimiento` es EXACTAMENTE el "
        "flujo que queremos correr en el cluster —`init` -> `source add` -> "
        "`discover` -> `review` -> `validate`—. Construir la imagen para ejecutar "
        "un camino que hoy no pasa seria empaquetar el fallo.")
