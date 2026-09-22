#!/usr/bin/env python3
"""
MEDIDA · el proyecto (23 de septiembre), antes de decidir qué es.

La consola tiene Projects —crear, «Code Repository», elegir plantilla, guardar—
y nada de eso persiste: no falta un endpoint, falta el concepto. Hoy la única
unidad de persistencia del inquilino es **el árbol de su celda** (un repo) y la
única jerarquía dentro es `packages/<ns>/<kind>/`; el alcance de todo es la
celda. La pregunta que esto mide no es dónde guardar un proyecto, sino **si un
proyecto es un segundo registro o una segunda vista del mismo árbol**:

  §1  LA SUPERFICIE   qué pide Projects hoy, campo a campo y acción a acción
                      (`lib/projects/mock.ts`, `ProjectsHome`, los dos modales):
                      qué está cableado y qué es `notImplemented`
  §2  EL ÁRBOL        qué da gratis una carpeta del árbol: si compila en
                      cualquier sitio, qué manifiesto admite (`README.md`, un
                      `.yaml` sin kind, un `kind:` que OOS no conoce), si el
                      índice de assets la nombra (`carpeta`), y si un paquete
                      puede vivir dentro de otra carpeta
  §3  EL ALCANCE      cuánto de la consola asume «celda» como único alcance:
                      rutas, ficheros, funciones de `lib/server`, y cuántas
                      llamadas de ore-serve llevarían un `proyecto` en medio
  §4  RAMAS           lo que hoy es del árbol ENTERO y no de un proyecto: dos
                      ramas que tocan dos proyectos distintos (¿funden limpio?),
                      una propuesta que toca los dos, y si un documento roto en
                      el proyecto B impide escribir en el proyecto A
  §5  LO DE DENTRO    los cinco productos de la tarjeta (Folder, Code
                      Repository, Pipeline, Lineage, Map): qué tiene backend hoy
                      y qué es lienzo
  §7  EL AISLAMIENTO  hasta dónde llega hoy si un proyecto fuera una carpeta:
                      qué es del árbol entero y no del proyecto —compilar, el
                      gobierno (conductos y retículos), los nombres, la sesión
                      de una persona, el índice— y qué costaría acotarlo
  §6  EL TAMAÑO       cuántos ítems y ficheros tendría un proyecto si fuera una
                      carpeta: lo que el índice de demo y victor ya dice

Uso:  python pruebas-de-fuego/medida-proyecto.py [--solo 1,2,3,4,5,6,7] [--consola C:/rubix-platform]
Necesita target/debug (ore, ore-serve), git. Todo en local; no toca el clúster.
No imprime ningún token.
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
PY = sys.executable
CONSOLA = sys.argv[sys.argv.index("--consola") + 1] if "--consola" in sys.argv else "C:/rubix-platform"
SOLO = set(sys.argv[sys.argv.index("--solo") + 1].split(",")) if "--solo" in sys.argv else {"1", "2", "3", "4", "5", "6", "7"}


def fila(a, b="", c=""):
    print("  %-54s %-20s %s" % (a, b, c))


def ms(t):
    return int((time.time() - t) * 1000)


def git(*args, cwd=None):
    r = subprocess.run(["git", *args], cwd=cwd, capture_output=True, text=True, encoding="utf-8",
                       env=dict(os.environ, GIT_AUTHOR_NAME="semilla", GIT_AUTHOR_EMAIL="s@x",
                                GIT_COMMITTER_NAME="semilla", GIT_COMMITTER_EMAIL="s@x"))
    return r.returncode, (r.stdout + r.stderr).strip()


def corre(args, env=None, cwd=None):
    r = subprocess.run(args, capture_output=True, text=True, encoding="utf-8", env=env or os.environ.copy(), cwd=cwd)
    return r.returncode, " ".join((r.stdout + "\n" + r.stderr).split())


def pide(base, metodo, ruta, cuerpo=None, cabeceras=None, sujeto="persona:ana"):
    datos = cuerpo.encode("utf-8") if isinstance(cuerpo, str) else (json.dumps(cuerpo).encode("utf-8") if cuerpo is not None else None)
    r = urllib.request.Request(base + ruta, data=datos, method=metodo)
    r.add_header("x-ore-sujeto", sujeto)
    r.add_header("content-type", "application/json")
    for k, v in (cabeceras or {}).items():
        r.add_header(k, v)
    try:
        with urllib.request.urlopen(r, timeout=120) as resp:
            t = resp.read().decode("utf-8")
            return resp.status, (json.loads(t) if t.strip().startswith(("{", "[")) else t)
    except urllib.error.HTTPError as e:
        t = e.read().decode("utf-8", "replace")
        try:
            return e.code, json.loads(t)
        except ValueError:
            return e.code, {"error": t.strip()[:200]}


def puerto_libre():
    import socket
    s = socket.socket(); s.bind(("127.0.0.1", 0)); p = s.getsockname()[1]; s.close(); return p


def lee(p):
    try:
        return open(p, encoding="utf-8").read()
    except OSError:
        return ""


# ── el árbol de la medida: dos «proyectos» como dos carpetas ────────────────
def arbol_semilla(d, con_proyectos=True):
    os.makedirs(d + "/datos", exist_ok=True)
    open(d + "/datos/pedidos.jsonl", "w").write("".join('{"id":"%d","pais":"ES"}\n' % i for i in range(1, 6)))
    open(d + "/ontology.config.yaml", "w").write(
        "apiVersion: oos.dev/v1alpha1\nkind: OntologyConfig\nmetadata: { name: medida, version: 0.1.0 }\n"
        "datasources:\n  - { name: ficheros, type: jsonl, connectionEnv: FICHEROS_DIR }\n")
    open(d + "/conduits.yaml", "w").write(
        "apiVersion: oos.dev/v1alpha1\nkind: ConduitPolicy\nmetadata: { name: m }\nspec:\n  owner: team:security\n"
        "  conduits:\n    materialization.payload: { oos.maturity: DRAFT }\n")
    for ns in ("ventas", "rrhh"):
        os.makedirs(d + "/packages/%s/tables" % ns, exist_ok=True)
        os.makedirs(d + "/packages/%s/views" % ns, exist_ok=True)
        open(d + "/packages/%s/package.yaml" % ns, "w").write(
            "apiVersion: oos.dev/v1alpha1\nkind: Package\nmetadata: { name: %s, version: 1.0.0, status: active, domain: %s }\nspec: { owner: team:%s }\n" % (ns, ns, ns))
        open(d + "/packages/%s/tables/pedidos.yaml" % ns, "w").write(
            "apiVersion: oos.dev/v1alpha8\nkind: Table\nmetadata: { name: pedidos, namespace: %s }\nspec:\n  datasource: ficheros\n  object: \"pedidos.jsonl\"\n"
            "  columns: { id: { type: Integer }, pais: { type: String } }\n  reads: { fullScan: cheap }\n  changes: { mode: append, witness: snapshot }\n" % ns)
        open(d + "/packages/%s/views/es.yaml" % ns, "w").write(
            "apiVersion: oos.dev/v1alpha12\nkind: View\nmetadata: { name: es, namespace: %s }\nspec:\n  owner: team:%s\n  from: { table: %s.pedidos }\n  fields: { id: id, pais: pais }\n" % (ns, ns, ns))


def main():
    for b in (ORE, SERVE):
        if not os.path.exists(b):
            print("falta", b, "— cargo build -p ore-cli -p ore-serve"); sys.exit(2)
    tmp = tempfile.mkdtemp(prefix="ore-proyecto-").replace("\\", "/")
    procs = []
    try:
        if "1" in SOLO:
            superficie()
        if "2" in SOLO:
            el_arbol(tmp)
        if "3" in SOLO:
            el_alcance()
        if "4" in SOLO:
            las_ramas(tmp, procs)
        if "5" in SOLO:
            lo_de_dentro()
        if "6" in SOLO:
            el_tamano()
        if "7" in SOLO:
            el_aislamiento(tmp, procs)
    finally:
        for p in procs:
            try:
                p.kill()
            except Exception:
                pass
        time.sleep(0.4)
        shutil.rmtree(tmp, ignore_errors=True)


# ── §1 · la superficie ──────────────────────────────────────────────────────
def superficie():
    print("§1 · la superficie: qué pide Projects hoy, y qué de eso está cableado")
    mock = lee(CONSOLA + "/lib/projects/mock.ts")
    campos = re.findall(r"^\s{2}(\w+):\s*([^;]+);", mock[mock.find("export interface Project"):mock.find("}", mock.find("export interface Project"))], re.M)
    fila("los campos de `Project` (mock.ts)", "%d" % len(campos), ", ".join("%s: %s" % (k, v.strip()) for k, v in campos)[:110])
    # Dónde viviría cada campo, si el proyecto fuera una carpeta del árbol.
    plano = {"id": "árbol (la ruta)", "name": "árbol", "description": "árbol", "collaborators": "control (ore-iam)",
             "createdAt": "árbol (git)", "updatedAt": "árbol (git)"}
    for k, _ in campos:
        fila("  %s" % k, plano.get(k, "?"))
    home = lee(CONSOLA + "/components/projects/ProjectsHome.tsx")
    detalle = lee(CONSOLA + "/components/projects/detail/ProjectDetailView.tsx")
    menu = lee(CONSOLA + "/components/projects/ProjectContextMenu.tsx")
    sin_hacer = re.findall(r"notImplemented\(['\"]([^'\"]+)['\"]\)", home + detalle + menu)
    fila("acciones `notImplemented`", "%d" % len(sin_hacer), ", ".join(sorted(set(sin_hacer)))[:110])
    fila("  lo que sí hace algo", "", "crear (añade a la lista local, en memoria)" if "SAMPLE_PROJECTS" in home else "?")
    crear = lee(CONSOLA + "/components/projects/detail/CreateResourceModal.tsx")
    productos = re.findall(r"id:\s*'([a-z-]+)',\s*\n\s*(?://[^\n]*\n\s*)*nombre:\s*'([^']+)'", crear)
    fila("lo que se crea DENTRO de un proyecto", "%d tarjetas" % len(productos), ", ".join(n for _, n in productos))
    plantilla = lee(CONSOLA + "/components/code-workspace/BuildPicker.tsx")
    ops = re.findall(r"id:\s*'([a-z-]+)'", plantilla)
    fila("las plantillas del «Code Repository» (BuildPicker)", "%d" % len(set(ops)), ", ".join(sorted(set(ops)))[:110])
    escribe = re.findall(r"(commitDelArbol|escribirFichero|PUT|acciones\.)", plantilla)
    fila("  ¿alguna escribe en el árbol?", "sí" if escribe else "no", "lo que hace: %s" % (", ".join(sorted(set(escribe))) if escribe else "elegir, y devolver la elección a quien abrió el modal"))
    print()


# ── §2 · el árbol: qué da gratis una carpeta ────────────────────────────────
def el_arbol(tmp):
    print("§2 · el árbol: qué da gratis una carpeta, y qué manifiesto admite")
    d = tmp + "/arbol"
    os.makedirs(d, exist_ok=True)
    arbol_semilla(d)
    env = dict(os.environ, FICHEROS_DIR=d + "/datos")
    c, s = corre([ORE, "validate", d], env)
    fila("el árbol semilla (dos paquetes)", "código %d" % c, s[:90] or "compila")

    # (a) una carpeta del cliente en medio: `packages/<ns>/<carpeta>/views/x.yaml`
    os.makedirs(d + "/packages/ventas/churn/views", exist_ok=True)
    shutil.move(d + "/packages/ventas/views/es.yaml", d + "/packages/ventas/churn/views/es.yaml")
    c, s = corre([ORE, "validate", d], env)
    fila("un documento en `packages/ventas/churn/views/`", "código %d" % c, s[:90] or "compila: la carpeta del cliente no estorba")

    # (b) el manifiesto: README.md, un .yaml sin kind, un kind que OOS no conoce
    open(d + "/packages/ventas/churn/README.md", "w").write("# Churn\n\nEl proyecto de abandono.\n")
    c, s = corre([ORE, "validate", d], env)
    fila("  + `README.md` como manifiesto", "código %d" % c, s[:90] or "compila: un README se ignora")
    open(d + "/packages/ventas/churn/proyecto.yaml", "w").write("nombre: churn\ndescripcion: el proyecto de abandono\n")
    c, s = corre([ORE, "validate", d], env)
    fila("  + `proyecto.yaml` SIN kind", "código %d" % c, (re.search(r"OOS\d{4}", s).group(0) + " " + s[:70]) if re.search(r"OOS\d{4}", s) else s[:90] or "compila")
    os.remove(d + "/packages/ventas/churn/proyecto.yaml")
    open(d + "/packages/ventas/churn/proyecto.yaml", "w").write(
        "apiVersion: oos.dev/v1alpha12\nkind: Project\nmetadata: { name: churn, namespace: ventas }\nspec: { owner: team:ventas }\n")
    c, s = corre([ORE, "validate", d], env)
    cod = re.search(r"OOS\d{4}", s)
    fila("  + `kind: Project` (que OOS no conoce)", "código %d" % c, (cod.group(0) + " · " + s[:70]) if cod else s[:90] or "compila")
    os.remove(d + "/packages/ventas/churn/proyecto.yaml")

    # (c) un paquete DENTRO de una carpeta de proyecto: `proyectos/churn/packages/<ns>/`
    d2 = tmp + "/arbol2"
    os.makedirs(d2, exist_ok=True)
    arbol_semilla(d2)
    os.makedirs(d2 + "/proyectos/churn", exist_ok=True)
    shutil.move(d2 + "/packages/ventas", d2 + "/proyectos/churn/ventas")
    env2 = dict(os.environ, FICHEROS_DIR=d2 + "/datos")
    c, s = corre([ORE, "validate", d2], env2)
    fila("un paquete movido a `proyectos/churn/ventas/`", "código %d" % c, s[:100] or "compila: un paquete fuera de `packages/`")
    c, s = corre([ORE, "assets", d2, "--json"], env2)
    try:
        j = json.loads(s[s.index("{"):])
        ns = sorted({i.get("paquete") or "(ninguno)" for i in j["items"].values()})
        fila("  el índice tras moverlo", "%d ítems" % len(j["items"]), "paquetes: %s" % ", ".join(ns))
        fila("  ¿sigue estando `ventas`?", "no" if not any("ventas" == (i.get("paquete") or "") for i in j["items"].values()) else "sí",
             "un paquete sólo se encuentra en `packages/<ns>/`: el proyecto NO puede estar por encima")
    except Exception as e:
        fila("  el índice", "", str(e)[:100])

    # (d) la carpeta en el índice: `carpeta` de cada ítem (0034 ④)
    c, s = corre([ORE, "assets", d, "--json"], env)
    try:
        j = json.loads(s[s.index("{"):])
        carpetas = sorted({i.get("carpeta", "") for i in j["items"].values()})
        fila("la carpeta que el índice ya da por ítem", "%d ítems" % len(j["items"]), "carpetas: %s" % ", ".join(repr(x) for x in carpetas))
        fila("  ¿bastaría para filtrar por proyecto?", "", "sí si el proyecto ES la carpeta; no dice de quién es ni cuándo nació")
    except Exception as e:
        fila("el índice", "", str(e)[:100])
    print()


# ── §3 · el alcance ─────────────────────────────────────────────────────────
def el_alcance():
    print("§3 · el alcance: cuánto de la consola asume la celda como único alcance")

    def cuenta(patron, ruta, glob=None):
        args = ["git", "grep", "-l", "-E", patron]
        if glob:
            args += ["--", glob]
        r = subprocess.run(args, cwd=CONSOLA, capture_output=True, text=True, encoding="utf-8", errors="replace")
        return [l for l in (r.stdout or "").strip().splitlines() if l]

    rutas = []
    base = CONSOLA + "/app/(workspace)/clusters/[celda]"
    for dirpath, _, ficheros in os.walk(base):
        for f in ficheros:
            if f in ("page.tsx", "layout.tsx"):
                rutas.append(os.path.relpath(os.path.join(dirpath, f), base).replace("\\", "/"))
    fila("rutas bajo `clusters/[celda]/`", "%d" % len(rutas), ", ".join(sorted(r.rsplit("/", 1)[0] for r in rutas if "/" in r))[:100])
    fila("  y `projects` es una de ellas", "", "sí: el proyecto hoy es una PÁGINA de la celda, no un alcance" if any(r.startswith("projects/") for r in rutas) else "no")
    fs = cuenta(r"\bcelda\b", CONSOLA, "*.ts*")
    fila("ficheros que nombran `celda`", "%d" % len(fs), ", ".join(sorted({f.split("/")[0] for f in fs}))[:100])
    srv = [f for f in os.listdir(CONSOLA + "/lib/server") if f.endswith(".ts")] if os.path.isdir(CONSOLA + "/lib/server") else []
    con_proyecto = cuenta(r"\bproyecto\b|\bprojectId\b", CONSOLA, "lib/server/*.ts")
    fila("módulos de `lib/server`", "%d" % len(srv), "con noción de proyecto: %d" % len(con_proyecto))
    q = lee(CONSOLA + "/lib/server/query.ts")
    rutasq = re.findall(r"ruta:\s*\([^)]*\)\s*=>\s*`([^`]+)`", q) + re.findall(r"ruta:\s*'([^']+)'", q)
    fila("llamadas a ore-serve declaradas en `query.ts`", "%d" % len(rutasq), "ninguna lleva proyecto" if not any("proyecto" in r for r in rutasq) else "alguna lleva proyecto")
    # Y en ORE: ¿alguna ruta tiene alcance por debajo de la celda?
    r = subprocess.run(["git", "grep", "-c", "-E", r"\(\"(GET|PUT|POST|DELETE)\", \[", "--", "crates/ore-serve/src/rutas.rs"], cwd=RAIZ, capture_output=True, text=True)
    fila("rutas de ore-serve (rutas.rs)", (r.stdout or "").strip().split(":")[-1] or "?", "el alcance es el árbol de la celda; no hay `proyecto` en ninguna")
    print()


# ── §4 · ramas y propuestas, que son del árbol entero ───────────────────────
def las_ramas(tmp, procs):
    print("§4 · ramas y propuestas: lo que hoy es del árbol ENTERO y no de un proyecto")
    forja = tmp + "/forja.git"
    git("init", "-q", "--bare", "-b", "main", forja)
    semilla = tmp + "/semilla"
    git("clone", "-q", forja, semilla)
    arbol_semilla(semilla)
    # dos «proyectos»: cada paquete en su carpeta
    os.makedirs(semilla + "/packages/ventas/churn/views", exist_ok=True)
    os.makedirs(semilla + "/packages/rrhh/nomina/views", exist_ok=True)
    shutil.move(semilla + "/packages/ventas/views/es.yaml", semilla + "/packages/ventas/churn/views/es.yaml")
    shutil.move(semilla + "/packages/rrhh/views/es.yaml", semilla + "/packages/rrhh/nomina/views/es.yaml")
    open(semilla + "/packages/ventas/churn/README.md", "w").write("# Churn\n")
    open(semilla + "/packages/rrhh/nomina/README.md", "w").write("# Nomina\n")
    git("add", "-A", cwd=semilla); git("commit", "-qm", "semilla", cwd=semilla); git("push", "-q", "origin", "HEAD:main", cwd=semilla)
    fila("el árbol: dos proyectos, dos carpetas", "", "packages/ventas/churn · packages/rrhh/nomina")

    # (a) dos ramas, cada una en su proyecto: ¿funden limpio?
    for nombre, ruta, texto in (("ana/churn", "packages/ventas/churn/views/es.yaml", "  where: { pais: ES }\n"),
                                ("bea/nomina", "packages/rrhh/nomina/views/es.yaml", "  where: { pais: PT }\n")):
        w = tmp + "/w-" + nombre.replace("/", "-")
        git("clone", "-q", forja, w)
        git("checkout", "-q", "-b", nombre, cwd=w)
        open(w + "/" + ruta, "a").write(texto)
        git("add", "-A", cwd=w); git("commit", "-qm", "en " + nombre, cwd=w); git("push", "-q", "origin", nombre, cwd=w)
    m = tmp + "/fusion"
    git("clone", "-q", forja, m)
    c1, s1 = git("merge", "--no-edit", "-q", "origin/ana/churn", cwd=m)
    c2, s2 = git("merge", "--no-edit", "-q", "origin/bea/nomina", cwd=m)
    fila("dos ramas que tocan dos proyectos distintos", "funden: %s" % ("sí" if c1 == 0 and c2 == 0 else "no"), (s1 + " " + s2)[:90])
    # (b) el espacio de ramas es uno: ¿se ve la rama del otro proyecto?
    c, s = git("ls-remote", "--heads", forja, cwd=m)
    ramas = [l.split("refs/heads/")[-1] for l in s.splitlines() if "refs/heads/" in l]
    fila("  el espacio de ramas", "%d ramas" % len(ramas), ", ".join(ramas) + "  ← una sola lista para todos los proyectos")
    # (c) una propuesta que toca los dos proyectos: nada lo impide
    w = tmp + "/w-cruzada"
    git("clone", "-q", forja, w)
    git("checkout", "-q", "-b", "cruzada", cwd=w)
    open(w + "/packages/ventas/churn/views/es.yaml", "a").write("# tocada\n")
    open(w + "/packages/rrhh/nomina/views/es.yaml", "a").write("# tocada\n")
    git("add", "-A", cwd=w); git("commit", "-qm", "toca los dos", cwd=w)
    c, s = git("diff", "--name-only", "origin/main", "HEAD", cwd=w)
    tocados = [l for l in s.splitlines() if l.strip()]
    proyectos = sorted({"/".join(l.split("/")[:3]) for l in tocados})
    fila("una rama que toca los dos proyectos", "%d ficheros" % len(tocados), "proyectos tocados: %s  ← nada lo impide" % ", ".join(proyectos))

    # (d) ¿un documento roto en el proyecto B impide escribir en el A?
    env = dict(os.environ, FICHEROS_DIR=semilla + "/datos", FORJA_TOKEN="no-hace-falta", PATH=BIN + os.pathsep + os.environ["PATH"])
    puerto = puerto_libre()
    base = "http://127.0.0.1:%d" % puerto
    srv = subprocess.Popen([SERVE, "--forja", "file://" + forja, "--ore", ORE, "--bind", "127.0.0.1:%d" % puerto,
                            "--identidad", "cabecera", "--no-es-produccion", "--organizacion", "demo"],
                           env=env, stdout=open(tmp + "/serve.log", "w"), stderr=subprocess.STDOUT)
    procs.append(srv)
    for _ in range(80):
        try:
            if pide(base, "GET", "/salud")[0] == 200:
                break
        except Exception:
            pass
        time.sleep(0.25)
    ROTA = "apiVersion: oos.dev/v1alpha12\nkind: View\nmetadata: { name: rota, namespace: rrhh }\nspec:\n  owner: team:rrhh\n  from: { table: rrhh.noExiste }\n  fields: { id: id }\n"
    c, r = pide(base, "PUT", "/arbol/packages/rrhh/nomina/views/rota.yaml", ROTA)
    fila("meter un documento ROTO en el proyecto B (por `/arbol`)", "HTTP %s" % c, (r.get("error", "") if isinstance(r, dict) else str(r))[:90])
    # por git, para que quede aunque la puerta lo niegue
    w2 = tmp + "/w-rota"
    git("clone", "-q", forja, w2)
    open(w2 + "/packages/rrhh/nomina/views/rota.yaml", "w").write(ROTA)
    git("add", "-A", cwd=w2); git("commit", "-qm", "rota en B", cwd=w2); git("push", "-q", "origin", "HEAD:main", cwd=w2)
    c, s = corre([ORE, "validate", w2], dict(os.environ, FICHEROS_DIR=semilla + "/datos"))
    fila("  el árbol con B roto", "código %d" % c, (re.search(r"OOS\d{4}", s).group(0) if re.search(r"OOS\d{4}", s) else s[:60]) + "  ← `ore validate` es del árbol entero")
    BUENA = "apiVersion: oos.dev/v1alpha12\nkind: View\nmetadata: { name: nueva, namespace: ventas }\nspec:\n  owner: team:ventas\n  from: { table: ventas.pedidos }\n  fields: { id: id }\n"
    c, r = pide(base, "PUT", "/documentos/View/ventas/nueva", {"yaml": BUENA})
    fila("escribir en el proyecto A con B roto", "HTTP %s" % c, ("commit %s" % r.get("commit")) if c in (200, 201) else str(r)[:90])
    fila("  la regla", "", "«no empeorar»: lo roto de antes no bloquea, pero el árbol NO compila para nadie")
    c, r = pide(base, "GET", "/assets")
    if c == 200:
        rotos = [k for k, v in (r.get("items") or {}).items() if any(x.get("rota") for x in v.get("relaciones", []))]
        fila("  y el índice", "%d ítems" % len(r.get("items") or {}), "con relación rota: %d (los de B se ven desde A)" % len(rotos))
    print()


# ── §5 · lo de dentro ───────────────────────────────────────────────────────
def lo_de_dentro():
    print("§5 · los cinco productos de la tarjeta: qué tiene backend hoy")
    rutas_ore = lee(RAIZ + "/crates/ore-serve/src/rutas.rs")

    def hay(patron):
        return bool(re.search(patron, rutas_ore))

    for nombre, fichero, patron, nota in (
        ("Folder", "components/files/FilesView.tsx", r"\[\"arbol\"", "el árbol tiene carpetas: `PUT /arbol/<ruta>`"),
        ("Code Repository", "components/code-workspace/CodeWorkspaceClient.tsx", r"\[\"documentos\"", "el workspace sobre el árbol (0030) + `/puestos` (0031)"),
        ("Pipeline", "components/pipelines/PipelinesClient.tsx", r"\[\"trabajos\"\]", "lo más cerca: `POST /trabajos` (un fichero del árbol como Job)"),
        ("Lineage exploration", "components/catalog/ItemDetail.tsx", r"\[\"assets\"\]", "el índice ya da las relaciones en las dos direcciones (0034); pantalla propia, no hay"),
        ("Map", "components/map/MapClient.tsx", r"NADA", "nada: es lienzo"),
    ):
        p = CONSOLA + "/" + fichero
        fuente = lee(p) if os.path.isfile(p) else ""
        if not fuente and os.path.isdir(CONSOLA + "/" + fichero):
            fuente = "".join(lee(os.path.join(CONSOLA + "/" + fichero, f)) for f in os.listdir(CONSOLA + "/" + fichero))
        mock = "mock" in fuente.lower() or "SAMPLE_" in fuente
        fila(nombre, "consola: %s" % ("mock" if mock else "vive" if fuente else "(no está)"), ("ore-serve: sí · " if hay(patron) else "ore-serve: no · ") + nota)
    print()


# ── §6 · el tamaño ──────────────────────────────────────────────────────────
def el_tamano():
    print("§6 · el tamaño: lo que un proyecto tendría dentro, según lo ya medido")
    adr = lee(RAIZ + "/docs/decisions/0034-el-catalogo-de-assets.md")
    for que, patron in (("demo", r"\((\d+) KB, (\d+) ítems\), victor"), ("victor", r"victor \*\*1 362 / 50 ms\*\* \((\d+) KB, (\d+) ítems")):
        m = re.search(patron, adr)
        fila("el índice de %s (0034)" % que, (m.group(2) + " ítems") if m else "?", ("%s KB" % m.group(1)) if m else "")
    m = re.search(r"el árbol de demo \| \*\*(\d+) ficheros, (\d+) KB\*\*", lee(RAIZ + "/docs/decisions/0030-el-arbol-en-el-editor.md"))
    fila("el árbol de demo (0030)", (m.group(1) + " ficheros") if m else "?", (m.group(2) + " KB") if m else "")
    fila("  si el proyecto fuera una carpeta", "", "hoy demo tiene 1 carpeta de cliente (la raíz): TODO sería un proyecto")
    print()


# ── §7 · el aislamiento: hasta dónde llega ──────────────────────────────────
def el_aislamiento(tmp, procs):
    print("§7 · el aislamiento: qué es del árbol entero y no del proyecto")
    d = tmp + "/ais"
    os.makedirs(d, exist_ok=True)
    arbol_semilla(d)
    # dos proyectos, cada uno en su paquete
    for ns, pr in (("ventas", "churn"), ("rrhh", "nomina")):
        os.makedirs(d + "/packages/%s/%s/views" % (ns, pr), exist_ok=True)
        shutil.move(d + "/packages/%s/views/es.yaml" % ns, d + "/packages/%s/%s/views/es.yaml" % (ns, pr))
        open(d + "/packages/%s/%s/README.md" % (ns, pr), "w").write("# %s\n" % pr)
    env = dict(os.environ, FICHEROS_DIR=d + "/datos")

    # (a) COMPILAR: ¿se puede compilar sólo un proyecto?
    c, s1 = corre([ORE, "validate", d], env)
    fila("compilar el árbol entero", "código %d" % c, s1[:80] or "compila")
    c, s1 = corre([ORE, "validate", d + "/packages/ventas"], env)
    fila("compilar SÓLO el paquete del proyecto", "código %d" % c, s1[:110])
    c, s1 = corre([ORE, "validate", d + "/packages/ventas/churn"], env)
    fila("compilar SÓLO la carpeta del proyecto", "código %d" % c, s1[:110])
    fila("  la unidad de compilación", "", "el árbol: la config, el retículo y los conductos están en la raíz")

    # (b) EL GOBIERNO: ¿puede un proyecto tener el suyo, y qué alcance tiene?
    open(d + "/lattice.yaml", "w").write("apiVersion: oos.dev/v1alpha3\nkind: Lattice\nmetadata: { name: sensitivity, namespace: gdpr }\nspec:\n  levels: [none, low, high]\n")
    open(d + "/conduits.yaml", "w").write(
        "apiVersion: oos.dev/v1alpha1\nkind: ConduitPolicy\nmetadata: { name: raiz }\nspec:\n  owner: team:security\n"
        "  conduits:\n    materialization.payload: { oos.maturity: DRAFT, gdpr.sensitivity: high }\n")
    # el proyecto A pone SU política, más estrecha, dentro de su carpeta
    open(d + "/packages/ventas/churn/conduits.yaml", "w").write(
        "apiVersion: oos.dev/v1alpha1\nkind: ConduitPolicy\nmetadata: { name: churn }\nspec:\n  owner: team:ventas\n"
        "  conduits:\n    materialization.payload: { oos.maturity: DRAFT, gdpr.sensitivity: none }\n")
    # y los dos proyectos copian algo etiquetado `low` por su datasource
    open(d + "/ontology.config.yaml", "w").write(
        "apiVersion: oos.dev/v1alpha1\nkind: OntologyConfig\nmetadata: { name: medida, version: 0.1.0 }\n"
        "datasources:\n  - { name: ficheros, type: jsonl, connectionEnv: FICHEROS_DIR, labels: { gdpr.sensitivity: low } }\n")
    for ns, pr in (("ventas", "churn"), ("rrhh", "nomina")):
        os.makedirs(d + "/packages/%s/%s/datasets" % (ns, pr), exist_ok=True)
        open(d + "/packages/%s/%s/datasets/copia.yaml" % (ns, pr), "w").write(
            "apiVersion: oos.dev/v1alpha12\nkind: Dataset\nmetadata: { name: copia, namespace: %s }\nspec:\n  owner: team:%s\n  from: { table: %s.pedidos }\n" % (ns, ns, ns))
    c, s1 = corre([ORE, "validate", d], env)
    cods = sorted(set(re.findall(r"OOS\d{4}", s1)))
    quien = sorted({m for m in re.findall(r"packages/(\w+)/(\w+)/datasets/copia.yaml", s1.replace("\\", "/"))})
    fila("una política MÁS ESTRECHA en la carpeta del proyecto A", "código %d · %s" % (c, ", ".join(cods) or "compila"),
         "a quién alcanza: %s" % (", ".join("/".join(x) for x in quien) if quien else "a nadie"))
    fila("  la regla de `clearances`", "", "las políticas se COMBINAN por el mínimo en TODO el árbol: una política de proyecto no acota, estrecha a todos")
    os.remove(d + "/packages/ventas/churn/conduits.yaml")

    # (c) LOS NOMBRES: dos proyectos, el mismo nombre
    os.makedirs(d + "/packages/ventas/otro/views", exist_ok=True)
    open(d + "/packages/ventas/otro/views/es.yaml", "w").write(
        "apiVersion: oos.dev/v1alpha12\nkind: View\nmetadata: { name: es, namespace: ventas }\nspec:\n  owner: team:ventas\n  from: { table: ventas.pedidos }\n  fields: { id: id }\n")
    c, s1 = corre([ORE, "validate", d], env)
    cod = re.search(r"OOS\d{4}", s1)
    fila("dos proyectos del mismo paquete con una View `es` cada uno", "código %d" % c, (cod.group(0) + " · " + s1[s1.find("error"):][:80]) if cod else "compila: se pisan sin avisar")
    shutil.rmtree(d + "/packages/ventas/otro")
    fila("  el espacio de nombres", "", "`<paquete>.<nombre>` es del árbol: el proyecto NO lo parte (y el puntero es `datasets/<p>_<n>.json`, plano)")

    # (d) LA SESIÓN: ¿una por persona, o una por proyecto?
    forja = tmp + "/ais.git"
    git("init", "-q", "--bare", "-b", "main", forja)
    w = tmp + "/ais-semilla"
    git("clone", "-q", forja, w)
    for x in os.listdir(d):
        (shutil.copytree if os.path.isdir(d + "/" + x) else shutil.copy)(d + "/" + x, w + "/" + x)
    git("add", "-A", cwd=w); git("commit", "-qm", "semilla", cwd=w); git("push", "-q", "origin", "HEAD:main", cwd=w)
    cola = tmp + "/ais-cola.git"
    git("init", "-q", "--bare", "-b", "main", cola)
    cw = tmp + "/ais-cola"
    git("clone", "-q", cola, cw)
    subprocess.run([PY, RAIZ + "/malla/gen-inquilino.py", "demo", "--a", tmp + "/rendido"], capture_output=True)
    for f in ("plantilla-puesto.txt", "plantilla-capa.txt"):
        if os.path.exists(tmp + "/rendido/" + f):
            shutil.copy(tmp + "/rendido/" + f, cw + "/" + f)
    git("add", "-A", cwd=cw); git("commit", "-qm", "plantilla", cwd=cw); git("push", "-q", "origin", "HEAD:main", cwd=cw)
    puerto = puerto_libre()
    base = "http://127.0.0.1:%d" % puerto
    srv = subprocess.Popen([SERVE, "--forja", "file://" + forja, "--cola", "file://" + cola, "--ore", ORE, "--bind", "127.0.0.1:%d" % puerto,
                            "--identidad", "cabecera", "--no-es-produccion", "--organizacion", "demo"],
                           env=dict(env, FORJA_TOKEN="no-hace-falta", PATH=BIN + os.pathsep + os.environ["PATH"]),
                           stdout=open(tmp + "/ais-serve.log", "w"), stderr=subprocess.STDOUT)
    procs.append(srv)
    for _ in range(80):
        try:
            if pide(base, "GET", "/salud")[0] == 200:
                break
        except Exception:
            pass
        time.sleep(0.25)
    c1, r1 = pide(base, "POST", "/puestos", {"lenguaje": "python"})
    c2, r2 = pide(base, "POST", "/puestos", {"lenguaje": "python"})
    fila("ana abre un puesto para el proyecto A, y otro para el B", "%s / %s" % (c1, c2),
         "ids: %s vs %s  ← %s" % (r1.get("id"), r2.get("id"), "el MISMO puesto" if r1.get("id") == r2.get("id") else "dos"))
    fila("  la regla", "", "`id_de(persona, entorno)`: una sesión por persona y lenguaje, no por proyecto")
    c3, r3 = pide(base, "POST", "/puestos", {"lenguaje": "python", "rama": "ana/churn"})
    fila("  y con rama dicha", "%s" % c3, "id %s · rama %s  ← la rama no cambia el id" % (r3.get("id"), r3.get("rama")))

    # (e) EL ÍNDICE Y LA LECTURA: alcance del árbol
    c, r = pide(base, "GET", "/assets")
    if c == 200:
        items = r.get("items") or {}
        carp = sorted({i.get("carpeta", "") for i in items.values()})
        fila("el índice de assets", "%d ítems" % len(items), "carpetas: %s · una cabeza, un índice: no hay `proyecto`" % ", ".join(repr(x) for x in carp))
    src = lee(RAIZ + "/crates/ore-serve/src/puestos.rs")
    fila("la lectura desde un puesto (`datos`)", "", "resuelve cualquier `<paquete>.<nombre>` del árbol: el conducto decide POR ETIQUETA, no por proyecto")
    print()


if __name__ == "__main__":
    main()
