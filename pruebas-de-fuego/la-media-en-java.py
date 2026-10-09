"""LA MEDIA EN JAVA (0049 JM0): el laboratorio de la JVM, sin clúster y sin Google.

Una orden, en segundos:

  1  el banco de la media (`banco_media.py`: la celda y `ore-medios` de mentira),
     con la entrada de `conformidad.default.derivar` instalada;
  2  la imagen `puesto-jvm:1` DE PROD, fijada por su digest (que se imprime):
     el JDK, DuckDB, Arrow y sus jars son los que corren en un puesto. Se
     comprueba que lleva los jars que dice `puesto/jvm/jars.txt`: si no, la
     imagen no es la de este árbol y la prueba no vale;
  3  el SDK de ESTE árbol (`puesto/jvm/ore/*.java`) compilado dentro de ella, y
     el ejecutor de la suite (`conformidad-media-jvm/`) corriendo los casos de
     `conformidad/media/casos/` contra el banco.

Lo que dice al final: el humo (el puesto llega al banco), `N/51` y cuánto queda
pendiente por `op`, y lo que tardó. Sale con 1 si algo está mal; lo pendiente no
es un fallo.

    PYTHONUTF8=1 python pruebas-de-fuego/la-media-en-java.py [open | open-003]

`JM_IMAGEN` cambia la imagen (por defecto, la del registro que haya en local).
"""
import json
import os
import subprocess
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import banco_media as banco  # noqa: E402
import banco_conformidad  # noqa: E402

T0 = time.time()
RAIZ = banco.RAIZ
IMAGEN = os.environ.get("JM_IMAGEN", "europe-west1-docker.pkg.dev/project-8853a180-450d-47be-b83/ore/puesto-jvm:1")
filtro = sys.argv[1] if len(sys.argv) > 1 else ""

# 1 · el banco. Escucha en el bucle local: Docker Desktop lleva
#   `host.docker.internal` hasta él, y no se abre a la red de la máquina.
banco.arrancar(escucha="127.0.0.1", anuncio="host.docker.internal")
banco_conformidad.montar()   # JM1: la muestra de `conformidad/media`
MUESTRA = json.load(open(os.path.join(RAIZ, "conformidad", "media", "muestra.json"), encoding="utf-8"))
por_ruta = {i["path"]: i for i in MUESTRA["items"] if "path" in i}
for s in MUESTRA["colecciones"]["derivar"]["solo"]:
    ruta, _, version = s.partition("@")
    it = por_ruta[ruta]
    if "texto" in it:
        datos = it["texto"].encode()
    elif "base64" in it:
        import base64
        datos = base64.b64decode(it["base64"])
    else:
        datos = next(v for v in it["versiones"] if not version or v["id"] == version)["texto"].encode()
    banco.ENTRADA[ruta] = datos

# 1b · la celda de un Preview de Java que lee media, la que genera `builds.rs`
#   (JM2): se genera si falta (Rust en Docker, como siempre); la prueba 18 la
#   corre con el kernel del agente. Tras tocar el arnés: borrarla y volver.
CELDA = os.path.join(RAIZ, "target", "celdas", "indice-preview.jsh")
if not os.path.exists(CELDA):
    print("generando la celda de builds.rs (cargo test, en Docker)…", flush=True)
    os.makedirs(os.path.dirname(CELDA), exist_ok=True)
    subprocess.run(["docker", "run", "--rm", "-v", RAIZ.replace("\\", "/") + ":/src", "-v", "ore-cargo-registry:/usr/local/cargo/registry",
                    "-v", "ore-pruebas-t:/tt", "-e", "CARGO_TARGET_DIR=/tt/main",
                    "-e", "ORE_CELDA_JAVA_MEDIA=/src/target/celdas/indice-preview.jsh", "-w", "/src", "rust:1-bookworm",
                    "cargo", "test", "-q", "-p", "ore-serve", "--bin", "ore-serve", "la_celda_de_un_preview_de_java_que_lee_media"],
                   env=dict(os.environ, MSYS_NO_PATHCONV="1"), stdout=subprocess.DEVNULL)

# 2 · la imagen, por su digest: lo que se probó es exactamente esto.
r = subprocess.run(["docker", "image", "inspect", IMAGEN, "--format", "{{json .RepoDigests}} {{.Created}}"],
                   capture_output=True, text=True)
if r.returncode != 0:
    sys.exit("✗ no está la imagen %s en local: `docker pull` con la cuenta activa" % IMAGEN)
digests, creada = r.stdout.strip().split(" ", 1)
digest = (json.loads(digests) or [IMAGEN])[0]
print("imagen %s (creada %s)" % (digest, creada[:19]), flush=True)

# 3 · dentro: los jars, compilar este árbol encima, correr el ejecutor.
GUION = r"""
set -e
mal=0
while read -r g v; do
  case "$g" in \#*|"") continue;; esac
  j="$(basename "$g")-$v.jar"
  [ -f "/opt/ore/lib/$j" ] || { echo "  ✗ la imagen no lleva $j (puesto/jvm/jars.txt): no es la de este árbol"; mal=1; }
done < /src/puesto/jvm/jars.txt
[ -f /opt/ore/lib/duckdb_jdbc.jar ] || { echo "  ✗ la imagen no lleva duckdb_jdbc.jar"; mal=1; }
[ "$mal" = 0 ] || exit 1
t=$(date +%s%N)
javac -nowarn -Xlint:-options --release 21 -encoding UTF-8 -d /tmp/c -cp '/opt/ore/lib/*' \
  /src/puesto/jvm/ore/*.java /src/pruebas-de-fuego/conformidad-media-jvm/*.java \
  /src/pruebas-de-fuego/conformidad-media-jvm/ore/*.java 2>&1 | grep -v '^Note:' || true
[ -f /tmp/c/conformidad/Ejecutor.class ] || { echo "  ✗ no compila"; exit 1; }
echo "  javac $(( ($(date +%s%N) - t) / 1000000 )) ms"
exec java -XX:+UseSerialGC -XX:MaxRAMPercentage=50 --add-opens=java.base/java.nio=ALL-UNNAMED \
  -cp '/tmp/c:/opt/ore/lib/*' conformidad.Ejecutor /src/conformidad/media "$FILTRO"
"""
orden = ["docker", "run", "--rm", "--entrypoint", "sh",
         "-v", RAIZ.replace("\\", "/") + ":/src:ro",
         "-e", "ORE_SERVE=" + os.environ["ORE_SERVE"], "-e", "PUESTO=p1",
         "-e", "ORE_ALMACEN=dir:/tmp", "-e", "FILTRO=" + filtro,
         "-e", "JM_ESCALA=" + str(banco_conformidad.ESCALA),
         digest, "-c", GUION]
p = subprocess.run(orden, env=dict(os.environ, MSYS_NO_PATHCONV="1"))
print("en %.1f s" % (time.time() - T0))
sys.exit(p.returncode)
