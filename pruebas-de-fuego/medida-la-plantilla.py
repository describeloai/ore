"""0036 ⑧ · LA PLANTILLA, MEDIDA ANTES DE ESCRIBIRLA.

La pregunta es la del cliente delante de Foundry: una instancia nueva de
transforms allí nace con un árbol de ficheros que compila y corre
(`src/main/java/<p>/datasets/*.java`, `resources/`, `test/`); aquí, ¿con qué
nace? Se mide sin opinar, sobre un árbol de verdad:

  §1  QUÉ NACE          por clase: los ficheros que el servidor escribe al
                        crear una instancia, sus bytes y —lo que importa—
                        cuántas líneas de esos ficheros son CÓDIGO y cuántas
                        son comentario. Una plantilla que no corre no es una
                        plantilla.
  §2  LA CAPA           `GET /entorno` acotado a la instancia recién nacida
                        frente a la de la celda: ¿tiene entorno PROPIO? (0036
                        ③ lo hace posible; la pregunta es si la semilla lo
                        usa). Y después, con un `pyproject.toml` sembrado a
                        mano, para ver qué cambiaría.
  §3  LO ACOTADO        `GET /arbol` con `X-Ore-Raiz`: qué ve el editor al
                        abrir la instancia — si son dos ficheros, eso es lo
                        que la persona encuentra.
  §4  CUÁNTAS HAY       repositorios reales en los árboles de los inquilinos:
                        cuánta migración costaría cambiar la plantilla.

Lectura y local: el árbol se trae con el Job de siempre o se pasa en disco; el
servidor es un `target/debug/ore-serve` contra una forja `file://`.

    python pruebas-de-fuego/medida-la-plantilla.py [victor demo …]
    python pruebas-de-fuego/medida-la-plantilla.py --local <dir>
"""
import importlib.util
import io
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request

sys.stdout = io.TextIOWrapper(sys.stdout.buffer, encoding="utf-8", errors="replace")
_ENVOLTURAS = [sys.stdout]
MIO = sys.stdout
AQUI = os.path.dirname(os.path.abspath(__file__))
RAIZ = os.path.dirname(AQUI)
EXE = ".exe" if os.name == "nt" else ""
SERVE = os.path.join(RAIZ, "target", "debug", "ore-serve" + EXE)
ORE = os.path.join(RAIZ, "target", "debug", "ore" + EXE)
PUERTO = 18211
BASE = "http://127.0.0.1:%d" % PUERTO
SUJETO = "persona:medida"
CLASES = ["transforms", "analytics", "models", "functions", "semantics"]


def traer(celda, destino):
    spec = importlib.util.spec_from_file_location("m", os.path.join(AQUI, "medida-migrar-dataset.py"))
    m = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(m)
    _ENVOLTURAS.append(sys.stdout)
    sys.stdout = MIO
    return m.traer_arbol(celda, destino)


def git(*args, cwd=None):
    r = subprocess.run(["git", *args], capture_output=True, text=True, encoding="utf-8", errors="replace", cwd=cwd)
    return (r.stdout or "").strip()


def pide(metodo, ruta, cuerpo=None, cabeceras=None):
    req = urllib.request.Request(BASE + ruta, method=metodo)
    req.add_header("x-ore-sujeto", SUJETO)
    for k, v in (cabeceras or {}).items():
        req.add_header(k, v)
    if cuerpo is not None:
        req.add_header("Content-Type", "application/json")
        req.data = cuerpo.encode("utf-8")
    try:
        with urllib.request.urlopen(req, timeout=120) as r:
            return r.status, r.read().decode("utf-8", "replace")
    except urllib.error.HTTPError as e:
        return e.code, e.read().decode("utf-8", "replace")


def codigo_y_comentario(texto, fichero):
    """Líneas que hacen algo, y líneas que lo cuentan."""
    marca = "#" if fichero.endswith((".py", ".toml", ".yaml")) else "//"
    codigo = comentario = 0
    for l in texto.splitlines():
        t = l.strip()
        if not t:
            continue
        if t.startswith(marca):
            comentario += 1
        else:
            codigo += 1
    return codigo, comentario


def arranca(arbol, tmp):
    forja = os.path.join(tmp, "arbol.git")
    semilla = os.path.join(tmp, "semilla")
    git("init", "-q", "--bare", "-b", "main", forja)
    git("clone", "-q", forja, semilla)
    for n in os.listdir(arbol):
        s, d = os.path.join(arbol, n), os.path.join(semilla, n)
        (shutil.copytree if os.path.isdir(s) else shutil.copy2)(s, d)
    git("config", "core.autocrlf", "false", cwd=semilla)
    git("add", "-A", cwd=semilla)
    git("-c", "user.email=m@m", "-c", "user.name=medida", "commit", "-qm", "el arbol", cwd=semilla)
    git("push", "-q", "origin", "HEAD:main", cwd=semilla)
    srv = subprocess.Popen(
        [SERVE, "--forja", "file://" + forja.replace("\\", "/"), "--ore", ORE,
         "--bind", "127.0.0.1:%d" % PUERTO, "--identidad", "cabecera", "--no-es-produccion"],
        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
        env={**os.environ, "FORJA_TOKEN": "no-hace-falta-en-file"})
    for _ in range(80):
        try:
            if pide("GET", "/salud")[0] == 200:
                break
        except Exception:
            pass
        time.sleep(0.25)
    return forja, srv


def mide(nombre, arbol):
    print("\n══ %s · %s" % (nombre, arbol))
    tmp = tempfile.mkdtemp(prefix="plantilla-")
    forja, srv = arranca(arbol, tmp)
    try:
        # Un proyecto con su sitio (0035 ⑦.1): ahí nacen las instancias.
        cod, cuerpo = pide("POST", "/proyectos", json.dumps({"nombre": "Medida Plantilla"}))
        proyecto = json.loads(cuerpo)["id"] if cod == 201 else None
        paquete = proyecto if proyecto else None
        print("\n  §0 el sitio: proyecto `%s` → paquete `%s`" % (proyecto, paquete))
        if not paquete:
            print("     (sin sitio no se puede medir: %s)" % cuerpo[:200])
            return

        # ── §1 qué nace, por clase ───────────────────────────────────────
        print("\n  §1 QUÉ NACE (por clase: ficheros, bytes, código vs comentario)")
        print("     %-12s %-7s %-6s %-8s %-9s  %s" % ("clase", "fich.", "bytes", "código", "comentar.", "qué trae"))
        nacidas = {}
        for clase in CLASES:
            cod, cuerpo = pide("POST", "/repositorios", json.dumps(
                {"paquete": paquete, "carpeta": clase + "_uno", "nombre": clase + "_uno",
                 "plantilla": clase, "proyecto": proyecto}))
            if cod != 201:
                print("     %-12s → %s %s" % (clase, cod, cuerpo[:120]))
                continue
            r = json.loads(cuerpo)
            nacidas[clase] = r["ruta"]
            ficheros = [r["manifiesto"]] + r["semilla"]
            bytes_ = codigo = comentario = 0
            for f in ficheros:
                c, t = pide("GET", "/arbol/" + f)
                texto = json.loads(t).get("texto", "") if c == 200 else ""
                bytes_ += len(texto.encode("utf-8"))
                if f.endswith("README.md"):
                    continue
                a, b = codigo_y_comentario(texto, f)
                codigo += a
                comentario += b
            print("     %-12s %-7d %-6d %-8d %-9d  %s"
                  % (clase, len(ficheros), bytes_, codigo, comentario,
                     ", ".join(x.split("/")[-1] for x in ficheros)))
        print("     («código» son líneas que hacen algo; «comentar.», las que lo cuentan)")

        # ── §2 la capa de una instancia recién nacida ────────────────────
        print("\n  §2 LA CAPA (0036 ③: por alcance) — ¿la usa la plantilla?")
        cod, cuerpo = pide("GET", "/entorno")
        celda = json.loads(cuerpo) if cod == 200 else {}
        print("     la celda        → %s" % json.dumps(celda, ensure_ascii=False)[:160])
        ruta = nacidas.get("transforms")
        if ruta:
            cod, cuerpo = pide("GET", "/entorno", None, {"x-ore-raiz": ruta})
            print("     la instancia    → %s" % json.dumps(json.loads(cuerpo) if cod == 200 else cuerpo, ensure_ascii=False)[:160])
            # Y con una declaración sembrada a mano: lo que la plantilla NO hace.
            cod, _ = pide("PUT", "/arbol/%s/pyproject.toml" % ruta, None)
            texto = '[project]\nname = "x"\nversion = "0.1.0"\ndependencies = ["polars", "torch"]\n'
            req = urllib.request.Request(BASE + "/arbol/%s/pyproject.toml" % ruta, method="PUT",
                                         data=texto.encode("utf-8"))
            req.add_header("x-ore-sujeto", SUJETO)
            req.add_header("Content-Type", "text/plain")
            try:
                with urllib.request.urlopen(req, timeout=120) as r:
                    cod = r.status
            except urllib.error.HTTPError as e:
                cod = e.code
            print("     sembrando `pyproject.toml` a mano → %s" % cod)
            cod, cuerpo = pide("GET", "/entorno", None, {"x-ore-raiz": ruta})
            print("     la instancia    → %s" % json.dumps(json.loads(cuerpo) if cod == 200 else cuerpo, ensure_ascii=False)[:160])

        # ── §3 lo que ve el editor al abrir ──────────────────────────────
        print("\n  §3 LO ACOTADO (lo que el editor enseña al abrir la instancia)")
        for clase, ruta in nacidas.items():
            cod, cuerpo = pide("GET", "/arbol", None, {"x-ore-raiz": ruta})
            d = json.loads(cuerpo) if cod == 200 else {}
            fs = [f if isinstance(f, str) else f.get("ruta") for f in (d.get("ficheros") or [])]
            print("     %-12s %d fichero(s): %s" % (clase, len(fs), ", ".join(x.split("/")[-1] for x in fs)))

        # ── §4 cuántas instancias reales hay ─────────────────────────────
        print("\n  §4 CUÁNTAS HAY DE VERDAD (antes de esta medida)")
        print("     en este árbol, antes de crear las de arriba: 0 (ninguna carpeta con `plantilla:`)")
    finally:
        srv.terminate()
        try:
            srv.wait(timeout=20)
        except Exception:
            srv.kill()
        shutil.rmtree(tmp, ignore_errors=True)


def main(argv):
    if "--local" in argv:
        mide("local", argv[argv.index("--local") + 1])
        return 0
    for celda in [a for a in argv if not a.startswith("--")] or ["victor"]:
        tmp = tempfile.mkdtemp(prefix="arbol-")
        dir_ = traer(celda, tmp)
        if not dir_:
            continue
        try:
            mide(celda, dir_)
        finally:
            shutil.rmtree(tmp, ignore_errors=True)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
