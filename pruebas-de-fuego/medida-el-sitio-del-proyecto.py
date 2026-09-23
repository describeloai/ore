"""0035 ⑦ · EL SITIO DEL PROYECTO, MEDIDO SOBRE EL ÁRBOL DE VERDAD.

La pregunta es la que hizo el cliente delante de la pantalla: «¿por qué veo
estas carpetas si mi proyecto está vacío, y por qué no puedo guardar ni un
item?». Se contesta sin opinar, sobre su árbol:

  §1  EL ÁRBOL          qué proyectos hay y qué nombra cada uno (`contiene`),
                        qué paquetes hay y QUÉ CARPETAS TIENEN DE VERDAD.
  §2  EL ÍNDICE         `GET /assets` contra un `ore-serve` sembrado con ese
                        mismo árbol: las `carpetas` de cada paquete — que es de
                        donde el asistente de la consola saca «Location».
  §3  LA CARPETA        «Create ▸ Folder» (`PUT /arbol/<p>/<c>/README.md`) y
                        después: ¿la nombra el índice? Antes de ⑦, NO — y por
                        eso no se podía elegir para guardar nada dentro.
  §4  GUARDAR           `POST /repositorios` tal cual lo manda el botón «Save»:
                        el código, el commit, la semilla y el `contiene` del
                        proyecto después.

Lectura y local: el árbol se trae con el Job de siempre (`traer_arbol`) o se
pasa ya en disco; el servidor es un `target/debug/ore-serve` contra una forja
`file://`. El clúster no cambia.

    python pruebas-de-fuego/medida-el-sitio-del-proyecto.py [victor demo …]
    python pruebas-de-fuego/medida-el-sitio-del-proyecto.py --local <dir>
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
PUERTO = 18207
BASE = "http://127.0.0.1:%d" % PUERTO
SUJETO = "persona:medida"
CARPETAS_DE_KIND = {"tables", "views", "datasets", "entities", "functions", "actions", "models", "interfaces", "concepts"}


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


def pide(metodo, ruta, cuerpo=None, tipo="application/json"):
    req = urllib.request.Request(BASE + ruta, method=metodo)
    req.add_header("x-ore-sujeto", SUJETO)
    if cuerpo is not None:
        req.add_header("Content-Type", tipo)
        req.data = cuerpo.encode("utf-8")
    try:
        with urllib.request.urlopen(req, timeout=120) as r:
            return r.status, r.read().decode("utf-8", "replace")
    except urllib.error.HTTPError as e:
        return e.code, e.read().decode("utf-8", "replace")


def carpetas_en_disco(dir_paquete):
    """Las carpetas que ESTÁN, quitando las del kind: lo que ⑦ dice que hay."""
    out = set()
    for raiz, dirs, _ in os.walk(dir_paquete):
        dirs[:] = [d for d in dirs if not d.startswith(".")]
        rel = os.path.relpath(raiz, dir_paquete).replace("\\", "/")
        if rel == ".":
            continue
        tramo = [c for c in rel.split("/") if c not in CARPETAS_DE_KIND]
        if tramo:
            out.add("/".join(tramo))
    return sorted(out)


def arranca(arbol, tmp):
    """Una forja pelada con ese árbol dentro, y un ore-serve contra ella."""
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
    tmp = tempfile.mkdtemp(prefix="sitio-")
    forja, srv = arranca(arbol, tmp)
    try:
        # ── §1 el árbol ──────────────────────────────────────────────────
        print("\n  §1 EL ÁRBOL")
        praiz = os.path.join(arbol, "proyectos")
        proyectos = sorted(os.listdir(praiz)) if os.path.isdir(praiz) else []
        if not proyectos:
            print("     proyectos/ : no hay")
        for n in proyectos:
            rm = os.path.join(praiz, n, "README.md")
            t = open(rm, encoding="utf-8", errors="replace").read() if os.path.isfile(rm) else ""
            cont = next((l.split(":", 1)[1].strip() for l in t.splitlines() if l.startswith("contiene:")), "(no lo dice)")
            print("     proyectos/%-24s contiene = %s" % (n, cont))
        praiz = os.path.join(arbol, "packages")
        paquetes = sorted(p for p in os.listdir(praiz) if os.path.isdir(os.path.join(praiz, p))) if os.path.isdir(praiz) else []
        for p in paquetes:
            print("     packages/%-26s carpetas de verdad = %s" % (p, carpetas_en_disco(os.path.join(praiz, p)) or "[]"))

        # ── §2 el índice ─────────────────────────────────────────────────
        print("\n  §2 EL ÍNDICE (`GET /assets`: de aquí sale «Location»)")
        cod, cuerpo = pide("GET", "/assets")
        d = json.loads(cuerpo) if cod == 200 else {}
        for p in d.get("paquetes", []):
            print("     %-28s carpetas = %-40s items = %s" % (p["name"], p.get("carpetas"), p.get("items")))
        print("     proyectos: %s" % json.dumps([{"nombre": p["nombre"], "contiene": p["contiene"]} for p in d.get("proyectos", [])], ensure_ascii=False))
        print("     repositorios: %d" % len(d.get("repositorios", [])))

        # ── §3 la carpeta recién creada ──────────────────────────────────
        print("\n  §3 LA CARPETA («Create ▸ Folder», y después el índice)")
        if not paquetes:
            print("     (sin paquetes: no hay dónde)")
        else:
            p0 = paquetes[0]
            cod, cuerpo = pide("PUT", "/arbol/packages/%s/medida_carpeta/README.md" % p0,
                               "# medida_carpeta\n\nQué va aquí.\n", "text/markdown")
            print("     PUT packages/%s/medida_carpeta/README.md → %s" % (p0, cod))
            print("     commit: %s" % git("--git-dir=" + forja, "log", "-1", "--format=%h · %s", "main"))
            cod, cuerpo = pide("GET", "/assets")
            d = json.loads(cuerpo) if cod == 200 else {}
            suyas = next((q.get("carpetas") for q in d.get("paquetes", []) if q["name"] == p0), None)
            print("     carpetas de %s = %s" % (p0, suyas))
            print("     ⇒ ¿se puede elegir para guardar dentro? %s"
                  % ("SÍ" if suyas and "medida_carpeta" in suyas else "NO — es invisible para el asistente"))

        # ── §4 guardar ───────────────────────────────────────────────────
        print("\n  §4 GUARDAR (`POST /repositorios`, lo que manda el botón «Save»)")
        if not paquetes:
            print("     (sin paquetes: no hay dónde)")
        else:
            p0 = paquetes[0]
            cuerpo_peticion = {"paquete": p0, "carpeta": "medida_repo", "nombre": "medida_repo", "plantilla": "transforms"}
            if proyectos:
                cuerpo_peticion["proyecto"] = proyectos[0]
            cod, cuerpo = pide("POST", "/repositorios", json.dumps(cuerpo_peticion))
            print("     POST /repositorios %s → %s" % (json.dumps(cuerpo_peticion, ensure_ascii=False), cod))
            if cod == 201:
                r = json.loads(cuerpo)
                print("     ruta = %s · semilla = %s" % (r["ruta"], r["semilla"]))
                print("     commit: %s" % git("--git-dir=" + forja, "log", "-1", "--format=%h %an · %s", "main"))
                for f in git("--git-dir=" + forja, "show", "--name-only", "--format=", "main").splitlines():
                    print("       %s" % f)
                if proyectos:
                    print("     el proyecto después:")
                    for l in git("--git-dir=" + forja, "show", "main:proyectos/%s/README.md" % proyectos[0]).splitlines():
                        print("       %s" % l)
            else:
                print("     %s" % cuerpo[:300])
    finally:
        srv.terminate()
        try:
            srv.wait(timeout=20)
        except Exception:
            srv.kill()
        shutil.rmtree(tmp, ignore_errors=True)


def main(argv):
    if "--local" in argv:
        d = argv[argv.index("--local") + 1]
        mide("local", d)
        return 0
    celdas = [a for a in argv if not a.startswith("--")] or ["demo", "victor"]
    for celda in celdas:
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
