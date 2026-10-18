#!/usr/bin/env python3
"""El build de Java compila el fichero del build y lo que usa, nada más (2026-10-12).

`Arnes.compilar` con un JDK 21 y los jars de `puesto/jvm/jars.txt` (los de
`la-media-en-java.py`, bajados una vez a `target/jm-jars`): un snippet de JShell
que nadie usa no tumba el build, lo usado de otra carpeta se compila, un fichero
roto que se usa falla con SU fichero y su línea, y `package` manda.
"""
import glob, os, shutil, subprocess, sys, urllib.request

RAIZ = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DUCKDB_JDBC = "https://repo1.maven.org/maven2/org/duckdb/duckdb_jdbc/1.5.6.0/duckdb_jdbc-1.5.6.0.jar"


def jars():
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


lib = jars()
clases = os.path.join(RAIZ, "target", "el-build-de-java")
shutil.rmtree(clases, ignore_errors=True)
fuentes = (glob.glob(os.path.join(RAIZ, "puesto", "jvm", "ore", "*.java"))
           + glob.glob(os.path.join(RAIZ, "pruebas-de-fuego", "el-build-de-java", "ore", "*.java")))
cp = os.path.join(lib, "*")
c = subprocess.run(["javac", "-nowarn", "-Xlint:-options", "--release", "21", "-encoding", "UTF-8", "-d", clases, "-cp", cp] + fuentes,
                   capture_output=True, text=True)
if c.returncode != 0:
    print(c.stdout + c.stderr)
    sys.exit("✗ no compila")
# El classpath del kernel (`Agente.Kernel.classpath()`) es el de esta JVM: el SDK y sus jars.
p = subprocess.run(["java", "-cp", clases + os.pathsep + cp, "ore.PruebaCompilar"])
sys.exit(p.returncode)
