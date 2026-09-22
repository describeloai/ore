#!/usr/bin/env python3
"""
MEDIDA · el repositorio, la unidad de trabajo (0035 ⑥), antes de construirlo.

Hoy el code workspace es **un entorno único sobre el árbol entero**: una sesión
por persona, una rama, y un editor que ve la celda completa. Lo que el producto
pide es **un conjunto de repositorios acotados** —muchos, con nombre y clase,
cada uno sobre su carpeta—, y para acotar hay que **persistir la instancia**.

Esta medida no pregunta si la idea es buena: pregunta **qué cuesta**, y lo mide
sobre los árboles de verdad.

  §1  LA FORMA        un `README.md` con `nombre` + `plantilla` dentro de una
                      carpeta de paquete, en demo y victor: ¿sigue invisible al
                      compilador? ¿cambia el índice? ¿cuánto cuesta recorrer el
                      árbol buscándolos?
  §2  QUIÉN Y CUÁNDO  qué dice git de una CARPETA (no de un fichero): si el
                      «last edited by / last edited» de la lista sale del árbol
                      tal cual, o hay que inventarlo
  §3  CUÁNTOS         cuántas carpetas serían repositorio hoy, y de qué tamaño
  §4  LO QUE ACOTA    el índice del editor, la sesión del puesto, la rama y las
                      propuestas: medido sin acotar, y ahora con `X-Ore-Raiz`
  §5  LA CONSOLA      la lista de la captura, columna a columna: qué campo la
                      llena y cuál no existe; y qué queda mock en el BuildPicker
  §7  LA CLASE        el techo y la versión (0036 ⑤): qué clase deja escribir,
                      cuál no abre sesión siquiera, y qué dice el índice de la
                      versión de la plantilla
  §6  LA CAPA         lo que 0036 ③ rompe: hoy la capa es la unión del árbol
                      entero —el `torch` de un repositorio lo bajan todas las
                      sesiones—; con alcance, cada repositorio tiene la suya

Uso:  python pruebas-de-fuego/medida-el-repositorio.py [--solo 1,2,3,4,5,6,7]
      [--celdas demo,victor] [--consola C:/rubix-platform]
Necesita target/debug (ore, ore-serve), git. §1–§3 leen las celdas; el resto, local.
"""
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request

sys.stdout.reconfigure(encoding="utf-8", errors="replace")
RAIZ = os.path.dirname(os.path.dirname(os.path.abspath(__file__))).replace("\\", "/")
BIN = RAIZ + "/target/debug"
EXE = ".exe" if os.name == "nt" else ""
ORE = "%s/ore%s" % (BIN, EXE)
SERVE = "%s/ore-serve%s" % (BIN, EXE)
CONSOLA = sys.argv[sys.argv.index("--consola") + 1] if "--consola" in sys.argv else "C:/rubix-platform"
CELDAS = sys.argv[sys.argv.index("--celdas") + 1].split(",") if "--celdas" in sys.argv else ["demo", "victor"]
SOLO = set(sys.argv[sys.argv.index("--solo") + 1].split(",")) if "--solo" in sys.argv else {"1", "2", "3", "4", "5", "6", "7"}
_ENVOLTURAS = [sys.stdout]

MANIFIESTO = """---
nombre: New Pipelines Java Transform
plantilla: transforms
---
Lo que este repositorio hace, en prosa.
"""


def fila(a, b="", c=""):
    print("  %-52s %-20s %s" % (a, b, c))


def git(*args, cwd=None):
    r = subprocess.run(["git", *args], cwd=cwd, capture_output=True, text=True, encoding="utf-8",
                       env=dict(os.environ, GIT_AUTHOR_NAME="semilla", GIT_AUTHOR_EMAIL="s@x",
                                GIT_COMMITTER_NAME="semilla", GIT_COMMITTER_EMAIL="s@x"))
    return r.returncode, (r.stdout + r.stderr).strip()


def corre(args, env=None, cwd=None):
    r = subprocess.run(args, capture_output=True, text=True, encoding="utf-8", env=env or os.environ.copy(), cwd=cwd)
    return r.returncode, " ".join((r.stdout + "\n" + r.stderr).split())


def lee(p):
    try:
        return open(p, encoding="utf-8").read()
    except OSError:
        return ""


def pide(base, metodo, ruta, cuerpo=None, sujeto="persona:ana", tipo="application/json"):
    datos = cuerpo.encode("utf-8") if isinstance(cuerpo, str) else (json.dumps(cuerpo).encode("utf-8") if cuerpo is not None else None)
    r = urllib.request.Request(base + ruta, data=datos, method=metodo)
    r.add_header("x-ore-sujeto", sujeto)
    r.add_header("content-type", tipo)
    t0 = time.time()
    try:
        with urllib.request.urlopen(r, timeout=120) as resp:
            t = resp.read().decode("utf-8")
            return resp.status, (json.loads(t) if t.strip().startswith(("{", "[")) else t), int((time.time() - t0) * 1000)
    except urllib.error.HTTPError as e:
        t = e.read().decode("utf-8", "replace")
        try:
            return e.code, json.loads(t), int((time.time() - t0) * 1000)
        except ValueError:
            return e.code, {"error": t.strip()[:200]}, int((time.time() - t0) * 1000)


def puerto_libre():
    import socket
    s = socket.socket(); s.bind(("127.0.0.1", 0)); p = s.getsockname()[1]; s.close(); return p


def trae_arbol(celda, destino):
    import importlib.util
    spec = importlib.util.spec_from_file_location("mm", RAIZ + "/pruebas-de-fuego/medida-migrar-dataset.py")
    m = importlib.util.module_from_spec(spec)
    _ENVOLTURAS.append(m)
    mio = sys.stdout
    mio.flush()
    spec.loader.exec_module(m)
    _ENVOLTURAS.append(sys.stdout)
    sys.stdout = mio
    return m.traer_arbol(celda, destino)


def indice_de(d, env=None):
    c, s = corre([ORE, "assets", d, "--json"], env)
    if c != 0:
        return None
    i = s.find("{")
    try:
        return json.loads(s[i:])
    except ValueError:
        return None


def paquetes_de(d):
    p = d + "/packages"
    return sorted([x for x in os.listdir(p) if os.path.isdir(p + "/" + x)]) if os.path.isdir(p) else []


# ── §1 · la forma, sobre los árboles de verdad ─────────────────────────────
def la_forma(tmp, arboles):
    print("§1 · la forma: un README con `nombre` + `plantilla` dentro de un paquete")
    for celda, d in arboles.items():
        ind = indice_de(d)
        if ind is None:
            fila("  %s" % celda, "no compila", "se salta")
            continue
        antes = len(ind.get("items") or {})
        pkgs = paquetes_de(d)
        if not pkgs:
            fila("  %s" % celda, "sin paquetes", "se salta")
            continue
        carpeta = d + "/packages/%s/raw" % pkgs[0]
        os.makedirs(carpeta, exist_ok=True)
        open(carpeta + "/README.md", "w", encoding="utf-8").write(MANIFIESTO)
        t0 = time.time()
        c, s = corre([ORE, "validate", d])
        ms = int((time.time() - t0) * 1000)
        ind2 = indice_de(d) or {}
        fila("  %s · `packages/%s/raw/README.md`" % (celda, pkgs[0]), "validate %d · %d ms" % (c, ms),
             "lo nombra: %s" % ("SÍ" if "raw" in s else "no — invisible al compilador"))
        fila("    y el índice", "%d ítems" % len(ind2.get("items") or {}),
             "igual" if len(ind2.get("items") or {}) == antes else "CAMBIA (%d antes)" % antes)
        # Lo que costaría encontrarlos: recorrer el árbol buscando READMEs.
        t0 = time.time()
        readmes = []
        for raiz_, _, fs in os.walk(d):
            if ".git" in raiz_.replace("\\", "/").split("/"):
                continue
            for f in fs:
                if f.lower() == "readme.md":
                    readmes.append(os.path.join(raiz_, f))
        cuantos = sum(1 for r in readmes if "plantilla:" in lee(r))
        fila("    recorrer el árbol buscándolos", "%d ms" % int((time.time() - t0) * 1000),
             "%d README en total · %d con `plantilla:`" % (len(readmes), cuantos))
        # La carpeta se va con él: si no, §3 contaría la que ha puesto §1.
        os.remove(carpeta + "/README.md")
        os.rmdir(carpeta)
    print()


# ── §2 · quién y cuándo, de una CARPETA ────────────────────────────────────
def quien_y_cuando(tmp, arboles):
    print("§2 · «last edited by / last edited» de una CARPETA, no de un fichero")
    for celda, d in arboles.items():
        pkgs = paquetes_de(d)
        if not pkgs:
            continue
        # El árbol viene sin `.git` (llega por un tar); se siembra uno para medir.
        w = tmp + "/git-" + celda
        shutil.rmtree(w, ignore_errors=True)
        shutil.copytree(d, w)
        git("init", "-q", "-b", "main", w)
        git("add", "-A", cwd=w)
        git("commit", "-qm", "semilla", cwd=w)
        carpeta = "packages/%s" % pkgs[0]
        os.makedirs(w + "/" + carpeta + "/raw", exist_ok=True)
        open(w + "/" + carpeta + "/raw/README.md", "w", encoding="utf-8").write(MANIFIESTO)
        git("add", "-A", cwd=w)
        r = subprocess.run(["git", "commit", "-qm", "el repositorio"], cwd=w, capture_output=True, text=True,
                           env=dict(os.environ, GIT_AUTHOR_NAME="Víctor Obregón Castro", GIT_AUTHOR_EMAIL="victor@x",
                                    GIT_COMMITTER_NAME="Víctor Obregón Castro", GIT_COMMITTER_EMAIL="victor@x"))
        t0 = time.time()
        c, s = git("log", "-1", "--format=%h\x1f%aI\x1f%an", "--", carpeta + "/raw", cwd=w)
        ms = int((time.time() - t0) * 1000)
        partes = s.split("\x1f")
        fila("  %s · `git log -1 -- <carpeta>`" % celda, "%d ms" % ms,
             "%s · %s · %s" % tuple(partes[:3]) if len(partes) >= 3 else s[:60])
        fila("    lo que la lista necesita", "", "NAME y la ruta son del manifiesto; LAST EDITED BY/LAST EDITED, de git")
        # Lo que ore-serve ya hace por fichero (`version_de`), ¿vale por carpeta?
        src = lee(RAIZ + "/crates/ore-serve/src/assets.rs")
        m = re.search(r'log", "-1", "--format=([^"]+)"', src)
        fila("    `version_de` en ore-serve", "el mismo `git log -1`", "formato %s — vale para una ruta cualquiera" % (m.group(1) if m else "?"))
    print()


# ── §3 · cuántos serían hoy ────────────────────────────────────────────────
def cuantos(arboles):
    print("§3 · cuántas carpetas serían repositorio hoy, y de qué tamaño")
    for celda, d in arboles.items():
        ind = indice_de(d) or {}
        carpetas = {}
        for it in (ind.get("items") or {}).values():
            if not it.get("paquete") or not it.get("carpeta"):
                continue
            k = "%s/%s" % (it["paquete"], it["carpeta"])
            carpetas[k] = carpetas.get(k, 0) + 1
        # Y las carpetas del disco que NO son de kind (las que podrían serlo).
        de_kind = {"tables", "views", "datasets", "entities", "functions", "actions", "models", "interfaces", "concepts"}
        candidatas = []
        for pkg in paquetes_de(d):
            base = d + "/packages/" + pkg
            for x in sorted(os.listdir(base)):
                if os.path.isdir(base + "/" + x) and x not in de_kind:
                    n = sum(len(fs) for _, _, fs in os.walk(base + "/" + x))
                    candidatas.append(("%s/%s" % (pkg, x), n))
        fila("  %s" % celda, "%d carpetas con ítems" % len(carpetas),
             ", ".join("%s (%d)" % kv for kv in sorted(carpetas.items())[:4]) or "ninguna")
        fila("    carpetas de cliente en disco", "%d" % len(candidatas),
             ", ".join("%s (%d ficheros)" % kv for kv in candidatas[:4]) or "ninguna: hoy no hay repositorios que listar")
    print()


# ── §4 · lo que hoy NO está acotado ────────────────────────────────────────
def lo_que_acota(tmp, procs):
    print("§4 · lo acotado (0036 ④): el editor, la sesión, la rama y las propuestas")
    forja = tmp + "/rep.git"
    git("init", "-q", "--bare", "-b", "main", forja)
    semilla = tmp + "/rep-semilla"
    git("clone", "-q", forja, semilla)
    for x in os.listdir(RAIZ + "/vendor/oos/examples/acme-retail"):
        o = RAIZ + "/vendor/oos/examples/acme-retail/" + x
        (shutil.copytree if os.path.isdir(o) else shutil.copy)(o, semilla + "/" + x)
    # dos «repositorios» dentro del mismo paquete
    for n in ("raw", "clean"):
        os.makedirs(semilla + "/packages/hr/" + n, exist_ok=True)
        open(semilla + "/packages/hr/%s/README.md" % n, "w", encoding="utf-8").write(
            MANIFIESTO.replace("New Pipelines Java Transform", "New %s Transform" % n))
    git("add", "-A", cwd=semilla); git("commit", "-qm", "acme + dos repos", cwd=semilla); git("push", "-q", "origin", "HEAD:main", cwd=semilla)
    # El puesto necesita la cola del inquilino (la plantilla del pod): sin ella,
    # `POST /puestos` es 503 y no se estaría midiendo lo que se quiere.
    cola = tmp + "/rep-cola.git"
    git("init", "-q", "--bare", "-b", "main", cola)
    cw = tmp + "/rep-cola"
    git("clone", "-q", cola, cw)
    subprocess.run([sys.executable, RAIZ + "/malla/gen-inquilino.py", "demo", "--a", tmp + "/rendido"], capture_output=True)
    for f in ("plantilla-puesto.txt", "plantilla-capa.txt"):
        if os.path.exists(tmp + "/rendido/" + f):
            shutil.copy(tmp + "/rendido/" + f, cw + "/" + f)
    git("add", "-A", cwd=cw); git("commit", "-qm", "plantilla", cwd=cw); git("push", "-q", "origin", "HEAD:main", cwd=cw)
    puerto = puerto_libre()
    base = "http://127.0.0.1:%d" % puerto
    srv = subprocess.Popen([SERVE, "--forja", "file://" + forja, "--cola", "file://" + cola, "--ore", ORE,
                            "--bind", "127.0.0.1:%d" % puerto, "--identidad", "cabecera", "--no-es-produccion",
                            "--organizacion", "demo"],
                           env=dict(os.environ, FORJA_TOKEN="no-hace-falta", PATH=BIN + os.pathsep + os.environ["PATH"]),
                           stdout=open(tmp + "/rep-serve.log", "w"), stderr=subprocess.STDOUT)
    procs.append(srv)
    for _ in range(80):
        try:
            if pide(base, "GET", "/salud")[0] == 200:
                break
        except Exception:
            pass
        time.sleep(0.25)

    def con_raiz(ruta, alcance):
        r = urllib.request.Request(base + ruta, method="GET")
        r.add_header("x-ore-sujeto", "persona:ana")
        if alcance:
            r.add_header("x-ore-raiz", alcance)
        try:
            with urllib.request.urlopen(r, timeout=60) as resp:
                t = resp.read().decode("utf-8")
                return resp.status, (json.loads(t) if t.strip().startswith(("{", "[")) else t)
        except urllib.error.HTTPError as e:
            return e.code, {"error": e.read().decode("utf-8", "replace")[:120]}

    # (a) el editor: el árbol de UNA carpeta
    c, r, ms = pide(base, "GET", "/arbol")
    todos = [f["ruta"] for f in (r or {}).get("ficheros", [])]
    dentro = [x for x in todos if x.startswith("packages/hr/raw/")]
    fila("`GET /arbol` sin cabecera (la celda)", "%d ficheros · %d ms" % (len(todos), ms),
         "del repositorio `hr/raw`: %d" % len(dentro))
    c2, r2 = con_raiz("/arbol", "packages/hr/raw")
    suyos = [f["ruta"] for f in (r2 or {}).get("ficheros", [])]
    fila("  con `X-Ore-Raiz: packages/hr/raw`", "%s · %d ficheros" % (c2, len(suyos)),
         "sólo lo suyo: %s · raíz dicha: %s" % (all(x.startswith("packages/hr/raw/") for x in suyos), (r2 or {}).get("raiz")))
    fila("  la cabeza", "%s" % str((r2 or {}).get("cabeza"))[:12],
         "la del árbol: %s  ← se acota QUÉ se lista, no de qué commit se habla" % ((r2 or {}).get("cabeza") == (r or {}).get("cabeza")))
    c2b, r2b = con_raiz("/arbol", "otra/cosa/aqui")
    c2c, r2c = con_raiz("/arbol", "packages/hr/noexiste")
    fila("  un alcance que no es carpeta de paquete · una que no está", "%s · %s" % (c2b, c2c), "422 y 404")

    # (b) la sesión: dos repos, dos puestos
    c3, p1, _ = pide(base, "POST", "/puestos", {"lenguaje": "python", "repositorio": "packages/hr/raw"})
    c4, p2, _ = pide(base, "POST", "/puestos", {"lenguaje": "python", "repositorio": "packages/hr/clean"})
    fila("ana abre un puesto en `hr/raw` y otro en `hr/clean`", "%s / %s" % (c3, c4),
         "ids: %s vs %s  ← %s" % (p1.get("id"), p2.get("id"), "el MISMO" if p1.get("id") == p2.get("id") else "DOS"))
    fila("  las ramas", "%s / %s" % (p1.get("rama"), p2.get("rama")),
         "una por persona y repositorio" if p1.get("rama") != p2.get("rama") else "la MISMA")
    c3b, p3, _ = pide(base, "POST", "/puestos", {"lenguaje": "python"})
    fila("  y sin repositorio", "%s" % p3.get("id"), "como siempre: uno por persona y entorno")

    # (c) las propuestas, acotadas
    c5, r5, _ = pide(base, "GET", "/propuestas")
    c6, r6 = con_raiz("/propuestas", "packages/hr/raw")
    fila("`GET /propuestas`", "%s" % c5,
         "con `X-Ore-Raiz`: %s · alcance dicho: %s" % (c6, (r6 or {}).get("alcance") if isinstance(r6, dict) else "—"))
    src = lee(RAIZ + "/crates/ore-serve/src/propuestas.rs")
    filtra = "api.ficheros(n)" in src.split("pub(crate) fn propuestas")[-1][:2000]
    fila("  ¿filtra por ruta?", "sí" if filtra else "no",
         "mira LOS FICHEROS, no el nombre de la rama: la pregunta es «¿esto cambia lo mío?»")
    print()


# ── §5 · la consola: la lista de la captura ────────────────────────────────
def la_consola():
    print("§5 · la lista de «Code repositories», columna a columna")
    columnas = [
        ("NAME", "`nombre` del manifiesto", "NO EXISTE hoy: una carpeta no tiene nombre bonito"),
        ("la ruta de debajo", "proyecto + `packages/<pkg>/<carpeta>`", "del índice (0035 ①)"),
        ("el icono", "`plantilla`", "NO EXISTE hoy: nada dice de qué plantilla nació"),
        ("LAST EDITED BY", "`git log -1 -- <carpeta>`", "ya está (§2)"),
        ("LAST EDITED", "lo mismo", "ya está (§2)"),
        ("pestaña Pull requests", "`/propuestas` de sus rutas", "existe sin filtro (§4)"),
        ("UPGRADE («Up to date»)", "versión de la plantilla", "no hay versiones de plantilla: se deja fuera"),
    ]
    for a, b, c in columnas:
        fila("  " + a, b, c)
    bp = lee(CONSOLA + "/components/code-workspace/BuildPicker.tsx")
    bloque = re.search(r"const PLANTILLAS[^=]*=\s*\[(.*?)\n\];", bp, re.S)
    ops = re.findall(r"id:\s*'([a-z-]+)'", bloque.group(1) if bloque else "")
    fila("las plantillas", "%d" % len(ops), ", ".join(ops))
    guardar = re.search(r"const guardar = \(\) =>([^;]+);", bp)
    fila("qué hace «Save» hoy", "navega", (guardar.group(1).strip() if guardar else "?")[:80])
    fila("  de dónde saca el proyecto y la carpeta", "de la URL", "`?project=&folder=` sobre `SAMPLE_PROJECT_FILES`")
    ws = lee(CONSOLA + "/app/(workspace)/clusters/[celda]/workspaces/page.tsx")
    fila("la ruta del workspace", "/workspaces", "una sola, sin repositorio: %d líneas" % len(ws.splitlines()))
    print()


# ── §6 · la capa: de la celda al repositorio ───────────────────────────────
def la_capa(tmp, procs):
    print("§6 · la capa: hoy es de la celda; con alcance, de cada repositorio")
    forja = tmp + "/capa.git"
    git("init", "-q", "--bare", "-b", "main", forja)
    semilla = tmp + "/capa-semilla"
    git("clone", "-q", forja, semilla)
    for x in os.listdir(RAIZ + "/vendor/oos/examples/acme-retail"):
        o = RAIZ + "/vendor/oos/examples/acme-retail/" + x
        (shutil.copytree if os.path.isdir(o) else shutil.copy)(o, semilla + "/" + x)
    # Lo de todos, lo del paquete, y lo de cada repositorio.
    open(semilla + "/pyproject.toml", "w").write("[project]\ndependencies = ['polars']\n")
    open(semilla + "/packages/hr/pyproject.toml", "w").write("[project]\ndependencies = ['duckdb']\n")
    for n, deps in (("modelos", "['torch']"), ("analisis", "[]")):
        d = semilla + "/packages/hr/" + n
        os.makedirs(d, exist_ok=True)
        open(d + "/README.md", "w", encoding="utf-8").write(
            MANIFIESTO.replace("New Pipelines Java Transform", "Repo " + n))
        open(d + "/pyproject.toml", "w").write("[project]\ndependencies = %s\n" % deps)
    git("add", "-A", cwd=semilla); git("commit", "-qm", "acme + dos repos con sus deps", cwd=semilla)
    git("push", "-q", "origin", "HEAD:main", cwd=semilla)
    puerto = puerto_libre()
    base = "http://127.0.0.1:%d" % puerto
    srv = subprocess.Popen([SERVE, "--forja", "file://" + forja, "--ore", ORE, "--bind", "127.0.0.1:%d" % puerto,
                            "--identidad", "cabecera", "--no-es-produccion"],
                           env=dict(os.environ, FORJA_TOKEN="no-hace-falta"),
                           stdout=open(tmp + "/capa-serve.log", "w"), stderr=subprocess.STDOUT)
    procs.append(srv)
    for _ in range(80):
        try:
            if pide(base, "GET", "/salud")[0] == 200:
                break
        except Exception:
            pass
        time.sleep(0.25)

    def entorno(alcance=None):
        r = urllib.request.Request(base + "/entorno", method="GET")
        r.add_header("x-ore-sujeto", "persona:ana")
        if alcance:
            r.add_header("x-ore-raiz", alcance)
        t0 = time.time()
        with urllib.request.urlopen(r, timeout=60) as resp:
            return json.loads(resp.read().decode("utf-8")), int((time.time() - t0) * 1000)

    celda, ms = entorno()
    fila("`GET /entorno` sin alcance (la celda)", "%d ms" % ms,
         "%s → %s" % (celda.get("declarado"), celda.get("digest")))
    uno, _ = entorno("packages/hr/modelos")
    dos, _ = entorno("packages/hr/analisis")
    fila("  el repositorio de modelos", uno.get("digest"), "%s" % uno.get("declarado"))
    fila("  el de análisis, al lado", dos.get("digest"), "%s" % dos.get("declarado"))
    fila("  ¿carga con el `torch` del vecino?",
         "NO" if "torch" not in (dos.get("declarado") or []) else "SÍ",
         "dos alcances, dos capas: %s" % ("distintas" if uno.get("digest") != dos.get("digest") else "LA MISMA"))
    c, r, _ = pide(base, "GET", "/entorno")
    fila("  y la de la celda sigue estando", celda.get("digest"), "lo de la raíz y el paquete es común a propósito")
    fila("  lo que esto arregla", "",
         "sin alcance, el `pyproject.toml` de un repositorio NO SE LEE (%s): sus deps tenían que subir al paquete, y ahí las baja todo el mundo"
         % ("torch fuera" if "torch" not in (celda.get("declarado") or []) else "?"))
    # Un alcance que no es una carpeta de paquete, y uno que no existe.
    for que, a in (("un alcance que no es una carpeta", "otra/cosa/aqui"), ("una carpeta que no está", "packages/hr/noexiste")):
        r2 = urllib.request.Request(base + "/entorno", method="GET")
        r2.add_header("x-ore-sujeto", "persona:ana")
        r2.add_header("x-ore-raiz", a)
        try:
            with urllib.request.urlopen(r2, timeout=60) as resp:
                cod = resp.status
        except urllib.error.HTTPError as e:
            cod = e.code
        fila("  %s" % que, "%s" % cod, a)
    print()


# ── §7 · la clase: el techo y la versión ───────────────────────────────────
def la_clase(tmp, procs):
    print("§7 · la clase: el techo (quién escribe, quién ni siquiera abre) y la versión")
    forja = tmp + "/clase.git"
    git("init", "-q", "--bare", "-b", "main", forja)
    semilla = tmp + "/clase-semilla"
    git("clone", "-q", forja, semilla)
    for x in os.listdir(RAIZ + "/vendor/oos/examples/acme-retail"):
        o = RAIZ + "/vendor/oos/examples/acme-retail/" + x
        (shutil.copytree if os.path.isdir(o) else shutil.copy)(o, semilla + "/" + x)
    git("add", "-A", cwd=semilla); git("commit", "-qm", "acme-retail", cwd=semilla)
    git("push", "-q", "origin", "HEAD:main", cwd=semilla)
    # La cola, para que `POST /puestos` no sea 503.
    cola = tmp + "/clase-cola.git"
    git("init", "-q", "--bare", "-b", "main", cola)
    cw = tmp + "/clase-cola"
    git("clone", "-q", cola, cw)
    subprocess.run([sys.executable, RAIZ + "/malla/gen-inquilino.py", "demo", "--a", tmp + "/rendido-clase"], capture_output=True)
    for f in ("plantilla-puesto.txt", "plantilla-capa.txt"):
        if os.path.exists(tmp + "/rendido-clase/" + f):
            shutil.copy(tmp + "/rendido-clase/" + f, cw + "/" + f)
    git("add", "-A", cwd=cw); git("commit", "-qm", "plantilla", cwd=cw); git("push", "-q", "origin", "HEAD:main", cwd=cw)
    puerto = puerto_libre()
    base = "http://127.0.0.1:%d" % puerto
    srv = subprocess.Popen([SERVE, "--forja", "file://" + forja, "--cola", "file://" + cola, "--ore", ORE,
                            "--bind", "127.0.0.1:%d" % puerto, "--identidad", "cabecera", "--no-es-produccion",
                            "--organizacion", "demo"],
                           env=dict(os.environ, FORJA_TOKEN="no-hace-falta", PATH=BIN + os.pathsep + os.environ["PATH"]),
                           stdout=open(tmp + "/clase-serve.log", "w"), stderr=subprocess.STDOUT)
    procs.append(srv)
    for _ in range(80):
        try:
            if pide(base, "GET", "/salud")[0] == 200:
                break
        except Exception:
            pass
        time.sleep(0.25)

    # Un repositorio de cada clase que importa aquí.
    for carpeta, plantilla in (("raw", "transforms"), ("mirar", "analytics"), ("modelo", "semantics")):
        c, r, _ = pide(base, "POST", "/repositorios",
                       {"paquete": "hr", "carpeta": carpeta, "nombre": carpeta.title(), "plantilla": plantilla})
        fila("crear `hr/%s` (%s)" % (carpeta, plantilla), "%s" % c, (r.get("error") or r.get("ruta") or "")[:60])

    # (a) el techo al abrir: la clase que no ejecuta no abre sesión
    for carpeta, que in (("raw", "transforms"), ("mirar", "analytics"), ("modelo", "semantics")):
        c, r, _ = pide(base, "POST", "/puestos", {"lenguaje": "python", "repositorio": "packages/hr/" + carpeta})
        fila("  abrir un puesto en `hr/%s` (%s)" % (carpeta, que), "%s" % c,
             ("id %s · escribe %s" % (r.get("id"), r.get("escribe"))) if c in (200, 201) else (r.get("error") or "")[:90])

    # (b) el techo al escribir: lo dice la ficha del puesto, y lo aplica el catálogo
    c, r, _ = pide(base, "GET", "/puestos")
    for p in (r or {}).get("puestos", []):
        fila("  %s" % p.get("id"), "%s" % p.get("plantilla"),
             "escribe: %s · repositorio: %s" % (p.get("escribe"), p.get("repositorio")))
    fila("  dónde se aplica", "", "`/v1` (el catálogo) y `datasets/…/confirmar`: donde ya se decide quién escribe")

    # (c) la versión: lo que el índice dice de la plantilla
    c, r, _ = pide(base, "GET", "/assets")
    for x in (r or {}).get("repositorios", []):
        fila("  %s" % x.get("ruta"), "%s v%s" % (x.get("plantilla"), x.get("plantillaVersion")),
             "la del producto: v%s · actualizable: %s" % (x.get("plantillaActual"), x.get("actualizable")))
    print()


def main():
    for b in (ORE, SERVE):
        if not os.path.exists(b):
            print("falta", b, "— cargo build -p ore-cli -p ore-serve"); sys.exit(2)
    tmp = tempfile.mkdtemp(prefix="ore-repositorio-").replace("\\", "/")
    procs = []
    arboles = {}
    try:
        if SOLO & {"1", "2", "3"}:
            for celda in CELDAS:
                d = trae_arbol(celda, tmp + "/" + celda)
                if d:
                    arboles[celda] = d
        if "1" in SOLO:
            la_forma(tmp, arboles)
        if "2" in SOLO:
            quien_y_cuando(tmp, arboles)
        if "3" in SOLO:
            cuantos(arboles)
        if "4" in SOLO:
            lo_que_acota(tmp, procs)
        if "5" in SOLO:
            la_consola()
        if "6" in SOLO:
            la_capa(tmp, procs)
        if "7" in SOLO:
            la_clase(tmp, procs)
    finally:
        for p in procs:
            try:
                p.kill()
            except Exception:
                pass
        time.sleep(0.4)
        shutil.rmtree(tmp, ignore_errors=True)


if __name__ == "__main__":
    main()
