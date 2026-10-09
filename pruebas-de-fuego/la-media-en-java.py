"""LA MEDIA EN JAVA (0049 JM0–JM5): el laboratorio de la JVM, sin clúster y sin Google.

Una orden, en segundos:

  1  el banco de la media (`banco_media.py` y `banco_conformidad.py`: la celda y
     `ore-medios` de mentira, con la muestra de `conformidad/media`);
  2  las celdas que genera `builds.rs` (un Preview que lee, un build que escribe,
     un Preview que deriva filas), si faltan, con `cargo test`;
  3  la JVM: el SDK de ESTE árbol (`puesto/jvm/ore/*.java`) compilado, y el
     ejecutor (`conformidad-media-jvm/`): las pruebas del SDK y la suite de
     `conformidad/media/casos/` contra el banco.

Dos modos:

- **en Docker** (por defecto, el laboratorio): la imagen `puesto-jvm:1` DE PROD,
  fijada por su digest (que se imprime), con el JDK, DuckDB y Arrow que corren en
  un puesto; se comprueba que lleva los jars de `puesto/jvm/jars.txt`, o la
  prueba no vale. Las celdas, con Rust en Docker.
- **local** (`JM_LOCAL=1`, el CI): el JDK 21 que haya, los jars de `jars.txt`
  bajados de Maven Central (como `el-puesto.sh`) a `target/jm-jars`, y `cargo`.

Lo que dice al final: las pruebas del SDK, el humo, `N/51` y lo pendiente por
`op`, y lo que tardó. Sale con 1 si algo está mal; lo pendiente no es un fallo.

    PYTHONUTF8=1 python pruebas-de-fuego/la-media-en-java.py [sdk | escrita | filas | open | open-003]

`JM_IMAGEN` cambia la imagen (por defecto, la del registro que haya en local).
"""
import glob
import json
import os
import shutil
import subprocess
import sys
import time
import urllib.request

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import banco_media as banco  # noqa: E402
import banco_conformidad  # noqa: E402

T0 = time.time()
RAIZ = banco.RAIZ
LOCAL = os.environ.get("JM_LOCAL") == "1"
IMAGEN = os.environ.get("JM_IMAGEN", "europe-west1-docker.pkg.dev/project-8853a180-450d-47be-b83/ore/puesto-jvm:1")
DUCKDB_JDBC = "https://repo1.maven.org/maven2/org/duckdb/duckdb_jdbc/1.5.5.1/duckdb_jdbc-1.5.5.1.jar"
filtro = sys.argv[1] if len(sys.argv) > 1 else ""

# 1 · el banco. Escucha en el bucle local; desde Docker, Docker Desktop lleva
#   `host.docker.internal` hasta él, y no se abre a la red de la máquina.
banco.arrancar(escucha="127.0.0.1", anuncio="127.0.0.1" if LOCAL else "host.docker.internal")
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

# 2 · las celdas de `builds.rs` (JM2, JM3, JM4b): se generan si falta alguna. Tras
#   tocar el arnés: borrarlas (`target/celdas/`) y volver.
CELDAS = {"ORE_CELDA_JAVA_MEDIA": "indice-preview.jsh", "ORE_CELDA_JAVA_BUILD_MEDIA": "paginas-build.jsh",
          "ORE_CELDA_JAVA_FILAS": "textos-preview.jsh"}
DIR_CELDAS = os.path.join(RAIZ, "target", "celdas")
if not all(os.path.exists(os.path.join(DIR_CELDAS, c)) for c in CELDAS.values()):
    print("generando las celdas de builds.rs (cargo test%s)…" % ("" if LOCAL else ", en Docker"), flush=True)
    os.makedirs(DIR_CELDAS, exist_ok=True)
    prueba = ["cargo", "test", "-q", "-p", "ore-serve", "--bin", "ore-serve", "la_celda_de_un_"]
    if LOCAL:
        r = subprocess.run(prueba, cwd=RAIZ, env=dict(os.environ, **{k: os.path.join(DIR_CELDAS, c) for k, c in CELDAS.items()}),
                           stdout=subprocess.DEVNULL)
    else:
        env = []
        for k, c in CELDAS.items():
            env += ["-e", "%s=/src/target/celdas/%s" % (k, c)]
        r = subprocess.run(["docker", "run", "--rm", "-v", RAIZ.replace("\\", "/") + ":/src", "-v", "ore-cargo-registry:/usr/local/cargo/registry",
                            "-v", "ore-pruebas-t:/tt", "-e", "CARGO_TARGET_DIR=/tt/main"] + env + ["-w", "/src", "rust:1-bookworm"] + prueba,
                           env=dict(os.environ, MSYS_NO_PATHCONV="1"), stdout=subprocess.DEVNULL)
    if r.returncode != 0:
        sys.exit("✗ las celdas de builds.rs no se generaron (cargo test salió con %d)" % r.returncode)

ENV_JVM = {"ORE_SERVE": os.environ["ORE_SERVE"], "PUESTO": "p1", "ORE_ALMACEN": "dir:/tmp",
           "FILTRO": filtro, "JM_ESCALA": str(banco_conformidad.ESCALA)}
ABRE = ["-XX:+UseSerialGC", "-XX:MaxRAMPercentage=50", "--add-opens=java.base/java.nio=ALL-UNNAMED"]


def jars_locales():
    """Los jars de la imagen, bajados una vez: DuckDB JDBC y los de `jars.txt`."""
    lib = os.environ.get("JM_JARS") or os.path.join(RAIZ, "target", "jm-jars")
    os.makedirs(lib, exist_ok=True)
    quiero = [("duckdb_jdbc.jar", DUCKDB_JDBC)]
    for linea in open(os.path.join(RAIZ, "puesto", "jvm", "jars.txt"), encoding="utf-8"):
        partes = linea.split()
        if len(partes) == 2 and not linea.startswith("#"):
            g, v = partes
            n = "%s-%s.jar" % (g.rsplit("/", 1)[-1], v)
            quiero.append((n, "https://repo1.maven.org/maven2/%s/%s/%s" % (g, v, n)))
    for n, url in quiero:
        f = os.path.join(lib, n)
        if not os.path.exists(f):
            with urllib.request.urlopen(url, timeout=120) as r, open(f + ".tmp", "wb") as o:
                shutil.copyfileobj(r, o)
            os.replace(f + ".tmp", f)
    return lib


if LOCAL:
    # 3 · local: el JDK del sistema y los jars de jars.txt.
    lib = jars_locales()
    clases = os.path.join(RAIZ, "target", "jm-clases")
    shutil.rmtree(clases, ignore_errors=True)
    fuentes = (glob.glob(os.path.join(RAIZ, "puesto", "jvm", "ore", "*.java"))
               + glob.glob(os.path.join(RAIZ, "pruebas-de-fuego", "conformidad-media-jvm", "*.java"))
               + glob.glob(os.path.join(RAIZ, "pruebas-de-fuego", "conformidad-media-jvm", "ore", "*.java")))
    cp = os.path.join(lib, "*")
    t = time.time()
    c = subprocess.run(["javac", "-nowarn", "-Xlint:-options", "--release", "21", "-encoding", "UTF-8", "-d", clases, "-cp", cp] + fuentes,
                       capture_output=True, text=True)
    if c.returncode != 0:
        print(c.stdout + c.stderr)
        sys.exit("  ✗ no compila")
    print("  javac %d ms · jdk local · jars de jars.txt en %s" % ((time.time() - t) * 1000, lib), flush=True)
    p = subprocess.run(["java"] + ABRE + ["-cp", clases + os.pathsep + cp, "conformidad.Ejecutor",
                                          os.path.join(RAIZ, "conformidad", "media"), filtro],
                       env=dict(os.environ, ORE_RAIZ=RAIZ, **ENV_JVM))
else:
    # 3 · en Docker: la imagen, por su digest: lo que se probó es exactamente esto.
    r = subprocess.run(["docker", "image", "inspect", IMAGEN, "--format", "{{json .RepoDigests}} {{.Created}}"],
                       capture_output=True, text=True)
    if r.returncode != 0:
        sys.exit("✗ no está la imagen %s en local: `docker pull` con la cuenta activa, o JM_LOCAL=1" % IMAGEN)
    digests, creada = r.stdout.strip().split(" ", 1)
    digest = (json.loads(digests) or [IMAGEN])[0]
    print("imagen %s (creada %s)" % (digest, creada[:19]), flush=True)
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
exec java """ + " ".join(ABRE) + r""" -cp '/tmp/c:/opt/ore/lib/*' conformidad.Ejecutor /src/conformidad/media "$FILTRO"
"""
    orden = ["docker", "run", "--rm", "--entrypoint", "sh", "-v", RAIZ.replace("\\", "/") + ":/src:ro", "-e", "ORE_RAIZ=/src"]
    for k, v in ENV_JVM.items():
        orden += ["-e", "%s=%s" % (k, v)]
    p = subprocess.run(orden + [digest, "-c", GUION], env=dict(os.environ, MSYS_NO_PATHCONV="1"))
print("en %.1f s" % (time.time() - T0))
sys.exit(p.returncode)
