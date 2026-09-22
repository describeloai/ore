#!/usr/bin/env python3
"""
MEDIDA · la consola de proyectos, antes de quitarle el mock (0035 ③).

0035 ② dejó los verbos (`POST/PUT/DELETE /proyectos`) y ① el índice con
`proyectos` en la raíz y en cada ítem. La consola sigue siendo mock. Esta
iteración se propone TRES cosas, y sólo tres:

    a) el listado de proyectos REALES
    b) crear y eliminar un proyecto
    c) crear y eliminar CARPETAS dentro de un proyecto

Lo de la siguiente (crear artefactos de code workspace con persistencia real)
se mide aquí sólo para saber dónde acaba ésta. Las secciones:

  §1  EL LISTADO      qué pinta `ProjectsHome` hoy, columna a columna, de dónde
                      sale cada una, y cuál de ellas `GET /assets` YA da
  §2  CREAR Y BORRAR  qué recoge el modal frente a lo que `POST /proyectos`
                      pide; qué hace hoy «Move to trash»; y si el árbol tiene
                      alguna noción de papelera (si no la tiene, borrar es
                      borrar, y hay que decirlo en la consola)
  §3  LAS CARPETAS    la pregunta cara: qué ES una carpeta dentro de un
                      proyecto. Se mide sobre un árbol de verdad: si git guarda
                      una carpeta vacía, si `PUT /arbol` admite un fichero que
                      no es un documento, si el índice la nombra, hasta qué
                      hondura, y qué pasa cuando el proyecto nombra DOS
                      paquetes (¿dónde cae la carpeta?)
  §4  EL COSTE        qué ficheros de la consola hay que tocar, cuáles tienen
                      WIP de otra sesión (no se tocan) y qué falta en
                      `query.ts`
  §5  LO QUE PERSISTE el viaje entero contra un `ore-serve` de verdad: crear,
                      listar, la carpeta, borrar — con lo que tarda cada uno

Uso:  python pruebas-de-fuego/medida-la-consola-de-proyectos.py [--solo 1,2,3,4,5]
      [--consola C:/rubix-platform]
Necesita target/debug (ore, ore-serve), git. Todo en local; no toca el clúster.
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
SOLO = set(sys.argv[sys.argv.index("--solo") + 1].split(",")) if "--solo" in sys.argv else {"1", "2", "3", "4", "5"}


def fila(a, b="", c=""):
    print("  %-52s %-18s %s" % (a, b, c))


def git(*args, cwd=None):
    r = subprocess.run(["git", *args], cwd=cwd, capture_output=True, text=True, encoding="utf-8",
                       env=dict(os.environ, GIT_AUTHOR_NAME="semilla", GIT_AUTHOR_EMAIL="s@x",
                                GIT_COMMITTER_NAME="semilla", GIT_COMMITTER_EMAIL="s@x"))
    return r.returncode, (r.stdout + r.stderr).strip()


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


# ── §1 · el listado ─────────────────────────────────────────────────────────
def el_listado():
    print("§1 · el listado: qué pinta Projects hoy y qué de eso da `/assets`")
    mock = lee(CONSOLA + "/lib/projects/mock.ts")
    home = lee(CONSOLA + "/components/projects/ProjectsHome.tsx")
    bloque = re.search(r"interface Project \{(.*?)\n\}", mock, re.S)
    campos = re.findall(r"^\s{2}(\w+)\??:", bloque.group(1) if bloque else "", re.M)
    fila("los campos de `Project` (mock.ts)", "%d" % len(campos), ", ".join(campos))
    filas = len(re.findall(r"^\s*project\('proj-", mock, re.M))
    fila("las filas de ejemplo", "%d" % filas, "`SAMPLE_PROJECTS`, en memoria; `ProjectsHome` las muta")
    cols = re.findall(r"<TableHeader[^>]*>\s*([^<]+?)\s*</TableHeader>", home)
    fila("las columnas de la tabla", "%d" % len(cols), ", ".join(c.strip() for c in cols) or "—")
    # De cada campo: de dónde saldría en lo que YA da `/assets` (0035 ①).
    de_donde = {
        "id": "`proyectos[].nombre` (el nombre de la carpeta)",
        "name": "`proyectos[].titulo`",
        "description": "`proyectos[].descripcion`",
        "createdAt": "`proyectos[].version.cuando` (el primer commit lo dirá `--diff-filter=A`)",
        "updatedAt": "`proyectos[].version.cuando`",
        "collaborators": "NO LO HAY: es ore-iam (⑤ 1)",
    }
    for c in campos:
        fila("  %s" % c, "del índice" if "NO" not in de_donde.get(c, "") else "no", de_donde.get(c, "?"))
    no_impl = re.findall(r"notImplemented\(`([^`]+)`", home) + re.findall(r"notImplemented\('([^']+)'", home)
    fila("acciones sin handler real", "%d" % len(no_impl), ", ".join(sorted({n.split('"')[0].strip() for n in no_impl})))
    fila("  lo único cableado", "", "«New project» → una lista en memoria (`setCreatedProjects`)")
    print()


# ── §2 · crear y borrar ─────────────────────────────────────────────────────
def crear_y_borrar():
    print("§2 · crear y borrar: el modal, el verbo, y qué es «trash» en un árbol")
    modal = lee(CONSOLA + "/components/projects/ProjectCreateModal.tsx")
    bloque = re.search(r"interface ProjectCreateValues \{(.*?)\n\}", modal, re.S)
    valores = re.findall(r"^\s{2}(\w+)\??:", bloque.group(1) if bloque else "", re.M)
    fila("lo que recoge el modal de crear", "%d campos" % len(set(valores)), ", ".join(sorted(set(valores))) or "—")
    fila("lo que pide `POST /proyectos` (0035 ②)", "3 campos", "nombre (obligatorio), descripcion?, contiene?")
    fila("  lo que falta preguntar", "contiene", "el modal no pregunta QUÉ nombra el proyecto: hoy nacería vacío")
    # El id: el del mock frente al que da el servidor
    fila("el `id` de hoy", "", "`proj-1` / `local-0-<nombre>`  ←  el servidor da `customer-churn` (del título)")
    # ¿Hay papelera en algún sitio?
    r = subprocess.run(["git", "grep", "-ilE", "papelera|trash", "--", "crates/"], cwd=RAIZ, capture_output=True, text=True)
    ficheros = [l for l in (r.stdout or "").splitlines() if l.strip()]
    fila("«papelera» en ORE (crates/)", "%d ficheros" % len(ficheros), ", ".join(ficheros[:3]) or "ninguno: en un árbol, borrar es un commit")
    trash = lee(CONSOLA + "/components/projects/ProjectContextMenu.tsx")
    acciones = re.findall(r"<MenuItem label=\"([^\"]+)\"", trash)
    fila("el menú de una fila", "%d acciones" % len(acciones), ", ".join(acciones) or "—")
    fila("  lo que esta iteración puede cablear", "3 de %d" % len(acciones),
         "Open (ya navega), Rename → `PUT /proyectos/{id}` y Move to trash → `DELETE`")
    fila("  y lo que hay que decir al borrar", "", "«se va la lente, no lo que nombraba» (`siguenEnElArbol`, 0035 ②)")
    print()


# ── §3 · las carpetas dentro de un proyecto ────────────────────────────────
def las_carpetas(tmp, procs):
    print("§3 · las carpetas dentro de un proyecto: qué son en un árbol de verdad")
    det = lee(CONSOLA + "/components/projects/detail/ProjectDetailView.tsx")
    fila("hoy, crear una carpeta", "en memoria", "`setFiles([... kind: 'folder', parentId ...])`, sobre `SAMPLE_PROJECT_FILES`")
    fila("  el árbol de la consola", "por `parentId`", "hondura libre; %d ficheros de ejemplo" % len(re.findall(r"^\s*file\('pf", lee(CONSOLA + "/lib/projects/files.ts"), re.M)))

    # (a) ¿git guarda una carpeta vacía?
    d = tmp + "/vacia"
    git("init", "-q", "-b", "main", d)
    os.makedirs(d + "/packages/hr/ingesta", exist_ok=True)
    open(d + "/x.txt", "w").write("x\n")
    git("add", "-A", cwd=d); git("commit", "-qm", "con una carpeta vacía", cwd=d)
    c, s = git("ls-tree", "-r", "--name-only", "HEAD", cwd=d)
    fila("una carpeta VACÍA en un commit", "%d ficheros" % len([x for x in s.splitlines() if x.strip()]),
         "la carpeta está: %s  ← una carpeta necesita un fichero" % ("sí" if "ingesta" in s else "NO"))

    # (b) un árbol servido: ¿qué admite `PUT /arbol` y qué nombra el índice?
    forja = tmp + "/forja.git"
    git("init", "-q", "--bare", "-b", "main", forja)
    semilla = tmp + "/semilla"
    git("clone", "-q", forja, semilla)
    for x in os.listdir(RAIZ + "/vendor/oos/examples/acme-retail"):
        o = RAIZ + "/vendor/oos/examples/acme-retail/" + x
        (shutil.copytree if os.path.isdir(o) else shutil.copy)(o, semilla + "/" + x)
    git("add", "-A", cwd=semilla); git("commit", "-qm", "acme-retail", cwd=semilla); git("push", "-q", "origin", "HEAD:main", cwd=semilla)
    puerto = puerto_libre()
    base = "http://127.0.0.1:%d" % puerto
    srv = subprocess.Popen([SERVE, "--forja", "file://" + forja, "--ore", ORE, "--bind", "127.0.0.1:%d" % puerto,
                            "--identidad", "cabecera", "--no-es-produccion"],
                           env=dict(os.environ, FORJA_TOKEN="no-hace-falta"),
                           stdout=open(tmp + "/serve.log", "w"), stderr=subprocess.STDOUT)
    procs.append(srv)
    for _ in range(80):
        try:
            if pide(base, "GET", "/salud")[0] == 200:
                break
        except Exception:
            pass
        time.sleep(0.25)

    c, r, _ = pide(base, "GET", "/assets")
    antes = len((r or {}).get("items") or {})
    paquetes = [p["name"] for p in (r or {}).get("paquetes", [])]
    fila("el árbol servido", "%d ítems" % antes, "paquetes: %s" % ", ".join(paquetes))

    # (b1) un README dentro de un paquete: ¿entra por /arbol?
    c1, r1, _ = pide(base, "PUT", "/arbol/packages/hr/ingesta/README.md", "# Ingesta\n", tipo="text/markdown")
    fila("`PUT /arbol/packages/hr/ingesta/README.md`", "%s" % c1,
         (r1.get("error") or str(r1))[:80] if c1 >= 300 else "entra: una carpeta se crea con un fichero dentro")
    # (b2) ¿la nombra el índice?
    c2, r2, _ = pide(base, "GET", "/assets")
    carpetas = sorted({i.get("carpeta", "") for i in (r2 or {}).get("items", {}).values()})
    hr = next((p for p in (r2 or {}).get("paquetes", []) if p["name"] == "hr"), {})
    fila("  ¿la nombra el índice?", "%d ítems" % len((r2 or {}).get("items") or {}),
         "carpetas del paquete hr: %s  ← el índice cuenta ÍTEMS, no carpetas" % (hr.get("carpetas")))
    c3, r3, _ = pide(base, "GET", "/arbol")
    rutas = [f["ruta"] for f in (r3 or {}).get("ficheros", [])]
    fila("  ¿la nombra `/arbol`?", "%d ficheros" % len(rutas),
         "está el README: %s  ← el editor SÍ la ve" % ("sí" if any("ingesta" in x for x in rutas) else "no"))
    # (b3) un documento dentro de la carpeta: ahora sí es una carpeta del índice
    vista = ("apiVersion: oos.dev/v1alpha8\nkind: View\nmetadata: { name: enIngesta, namespace: hr }\n"
             "spec:\n  owner: team:people-data\n  from: { view: empleados }\n  fields:\n    id: employeeId\n")
    c4, r4, _ = pide(base, "PUT", "/arbol/packages/hr/ingesta/views/enIngesta.yaml", vista, tipo="text/yaml")
    c5, r5, _ = pide(base, "GET", "/assets")
    hr2 = next((p for p in (r5 or {}).get("paquetes", []) if p["name"] == "hr"), {})
    fila("un documento dentro de la carpeta", "%s" % c4, "carpetas del paquete hr: %s" % (hr2.get("carpetas")))
    # (b4) hondura
    c6, _, _ = pide(base, "PUT", "/arbol/packages/hr/ingesta/2026/views/honda.yaml",
                    vista.replace("enIngesta", "honda"), tipo="text/yaml")
    c7, r7, _ = pide(base, "GET", "/assets")
    hr3 = next((p for p in (r7 or {}).get("paquetes", []) if p["name"] == "hr"), {})
    fila("  y a dos de hondura", "%s" % c6, "carpetas: %s  ← la hondura se guarda entera" % (hr3.get("carpetas")))
    # (b5) borrar la carpeta: ¿hay verbo?
    c8, r8, _ = pide(base, "DELETE", "/arbol/packages/hr/ingesta/2026/views/honda.yaml")
    c9, r9, _ = pide(base, "DELETE", "/arbol/packages/hr/ingesta")
    fila("borrar la carpeta", "fichero %s · carpeta %s" % (c8, c9),
         (r9.get("error") or "")[:70] or "hay verbo de carpeta")
    fila("  lo que se sigue", "", "borrar una carpeta = borrar sus ficheros: hoy es N llamadas, o un verbo nuevo")

    # (c) el proyecto que nombra DOS paquetes: ¿dónde cae la carpeta nueva?
    pide(base, "POST", "/proyectos", {"nombre": "Dos Paquetes", "contiene": ["hr", "sales"]})
    c10, r10, _ = pide(base, "GET", "/assets")
    p = next((x for x in (r10 or {}).get("proyectos", []) if x["nombre"] == "dos-paquetes"), {})
    fila("un proyecto que nombra dos paquetes", "%d ítems" % p.get("items", 0), "contiene: %s" % p.get("contiene"))
    fila("  «Create ▸ Folder» tendría que preguntar", "en cuál", "la carpeta vive DENTRO de un paquete: el proyecto no es un sitio")
    fila("  salvo que el proyecto nombre uno solo", "", "entonces no hay nada que preguntar: es el suyo")
    print()


# ── §4 · el coste en la consola ─────────────────────────────────────────────
def el_coste():
    print("§4 · el coste: qué se toca en la consola, y qué NO se puede tocar")
    ficheros = [
        "lib/projects/mock.ts", "lib/projects/files.ts",
        "components/projects/ProjectsHome.tsx", "components/projects/ProjectCreateModal.tsx",
        "components/projects/ProjectContextMenu.tsx",
        "components/projects/detail/ProjectDetailView.tsx",
        "components/projects/detail/CreateResourceModal.tsx",
        "components/projects/detail/ProjectFileContextMenu.tsx",
        "app/(workspace)/clusters/[celda]/projects/page.tsx",
        "app/(workspace)/clusters/[celda]/projects/[id]/page.tsx",
        "lib/server/query.ts",
    ]
    # Lo que otra sesión tiene a medias (modificado o sin seguir): NO se toca.
    r = subprocess.run(["git", "status", "--porcelain"], cwd=CONSOLA, capture_output=True, text=True)
    wip = {l[3:].strip().strip('"').replace("\\", "/") for l in (r.stdout or "").splitlines() if l.strip()}
    total = 0
    tocados = []
    for f in ficheros:
        n = len(lee(CONSOLA + "/" + f).splitlines())
        total += n
        marca = "WIP DE OTRA SESIÓN" if f in wip else ""
        if marca:
            tocados.append(f)
        fila("  " + f, "%d líneas" % n, marca)
    fila("a tocar", "%d ficheros · %d líneas" % (len(ficheros), total), "con WIP ajeno: %d" % len(tocados))
    q = lee(CONSOLA + "/lib/server/query.ts")
    tiene = [k for k in ("assets", "arbol", "proyectos") if re.search(r"\n\s*%s:\s*\{" % k, q)]
    fila("en `query.ts`", "declaradas: %s" % ", ".join(tiene), "falta: proyectos (POST/PUT/DELETE) y el PUT/DELETE de `/arbol`")
    esc = re.findall(r"metodo:\s*'(POST|PUT|DELETE)'", q)
    fila("  llamadas de escritura que ya hay", "%d" % len(esc), "la figura existe: una más es una fila de la tabla")
    print()


# ── §5 · el viaje entero, contra un servidor de verdad ─────────────────────
def lo_que_persiste(tmp, procs):
    print("§5 · el viaje entero: crear · listar · la carpeta · borrar (ms)")
    forja = tmp + "/f5.git"
    git("init", "-q", "--bare", "-b", "main", forja)
    semilla = tmp + "/s5"
    git("clone", "-q", forja, semilla)
    for x in os.listdir(RAIZ + "/vendor/oos/examples/acme-retail"):
        o = RAIZ + "/vendor/oos/examples/acme-retail/" + x
        (shutil.copytree if os.path.isdir(o) else shutil.copy)(o, semilla + "/" + x)
    git("add", "-A", cwd=semilla); git("commit", "-qm", "acme-retail", cwd=semilla); git("push", "-q", "origin", "HEAD:main", cwd=semilla)
    puerto = puerto_libre()
    base = "http://127.0.0.1:%d" % puerto
    srv = subprocess.Popen([SERVE, "--forja", "file://" + forja, "--ore", ORE, "--bind", "127.0.0.1:%d" % puerto,
                            "--identidad", "cabecera", "--no-es-produccion"],
                           env=dict(os.environ, FORJA_TOKEN="no-hace-falta"),
                           stdout=open(tmp + "/serve5.log", "w"), stderr=subprocess.STDOUT)
    procs.append(srv)
    for _ in range(80):
        try:
            if pide(base, "GET", "/salud")[0] == 200:
                break
        except Exception:
            pass
        time.sleep(0.25)
    pasos = [
        ("crear el proyecto", "POST", "/proyectos", {"nombre": "Customer Churn", "descripcion": "Abandono.", "contiene": ["hr"]}),
        ("listarlos (la misma llamada del catálogo)", "GET", "/assets", None),
        ("la carpeta dentro (un README)", "PUT", "/arbol/packages/hr/ingesta/README.md", "# Ingesta\n"),
        ("volver a listar", "GET", "/assets", None),
        ("borrar la carpeta (su fichero)", "DELETE", "/arbol/packages/hr/ingesta/README.md", None),
        ("borrar el proyecto", "DELETE", "/proyectos/customer-churn", None),
    ]
    for que, m, ruta, cuerpo in pasos:
        tipo = "text/markdown" if isinstance(cuerpo, str) else "application/json"
        c, r, ms = pide(base, m, ruta, cuerpo, tipo=tipo)
        extra = ""
        if isinstance(r, dict):
            if "proyectos" in r:
                extra = "proyectos: %s" % [x["nombre"] for x in r["proyectos"]]
            elif "siguenEnElArbol" in r:
                extra = "siguen en el árbol: %s" % r["siguenEnElArbol"]
            elif "commit" in r:
                extra = "commit %s" % str(r["commit"])[:8]
        fila("  " + que, "%s · %d ms" % (c, ms), extra)
    print()


def main():
    for b in (ORE, SERVE):
        if not os.path.exists(b):
            print("falta", b, "— cargo build -p ore-cli -p ore-serve"); sys.exit(2)
    tmp = tempfile.mkdtemp(prefix="ore-consola-proyectos-").replace("\\", "/")
    procs = []
    try:
        if "1" in SOLO:
            el_listado()
        if "2" in SOLO:
            crear_y_borrar()
        if "3" in SOLO:
            las_carpetas(tmp, procs)
        if "4" in SOLO:
            el_coste()
        if "5" in SOLO:
            lo_que_persiste(tmp, procs)
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
