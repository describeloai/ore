"""El repositorio de la sesión de Python, en disco: sus pruebas y sus tipos (ORE 0050 P4).

El gemelo, para Python, de lo que `puesto/node/correa.mjs` hace para TypeScript.
La correa del agente (`agente.py`) entrega aquí dos peticiones propias del
editor, que NO van a pyright ni esperan en su cola:

- ``ore/probar``: las pruebas del repositorio con pytest —lo guardado en la rama
  y, encima, lo que la consola manda sin guardar—, en un proceso aparte, SIN la
  identidad de la sesión y con su tope. Contesta lo mismo que Node:
  ``{repositorio, todos, ficheros, pruebas, resumen, registro, ms}``.
- ``ore/comprobar``: pyright sobre el repositorio entero tal como quedó en la
  rama (lo guardado), tras un commit: lo que un cambio rompe en ficheros que
  nadie tiene abiertos. Contesta ``{repositorio, diagnosticos, errores, ficheros, ms}``.

Y lo que las dos necesitan, que también le sirve a pyright mientras se escribe:

1. EL REPOSITORIO EN DISCO: su código y sus datos de prueba, leídos de
   `ore-serve` en la rama del puesto (``GET /arbol`` acotado a su carpeta, y cada
   fichero), en ``/trabajo/<ruta del repositorio>``. Sin él, `test_example.py` no
   puede importar `example`, ni pyright resolverlo.
2. EL ESPEJO: lo que el editor abre o cambia se escribe también en disco.
"""
import json
import os
import re
import shlex
import signal
import subprocess
import sys
import tempfile
import threading
import time
import urllib.parse

AQUI = os.path.dirname(os.path.abspath(__file__))

# Lo que se trae al disco: código y lo que una prueba suele leer.
DE_REPOSITORIO = re.compile(r"\.(py|pyi|toml|cfg|ini|json|csv|tsv|txt|yaml|yml|sql)$")
FICHEROS_MAXIMOS = 2000
BYTES_MAXIMOS = 1 << 20
# Lo más que se devuelve del registro: lo último, donde están los fallos y el resumen.
TOPE_REGISTRO = 256 * 1024
FUERA = {"__pycache__", "node_modules", ".venv", "venv"}


def log(*a):
    print("repositorio ·", *a, flush=True)


def es_de_prueba(nombre):
    """Lo que pytest descubre por defecto: `test_*.py` y `*_test.py`."""
    return nombre.endswith(".py") and (nombre.startswith("test_") or nombre.endswith("_test.py"))


def ficheros_que(pred, d, base=""):
    """Los ficheros de `d` (relativos a él) que cumplen `pred`, sin cachés ni ocultos."""
    if not os.path.isdir(d):
        return []
    fuera = []
    for e in sorted(os.listdir(d)):
        if e in FUERA or e.startswith("."):
            continue
        rel = "%s/%s" % (base, e) if base else e
        ruta = os.path.join(d, e)
        if os.path.isdir(ruta):
            fuera += ficheros_que(pred, ruta, rel)
        elif pred(e):
            fuera.append(rel)
    return sorted(fuera)


def en_disco(uri, trabajo):
    """El fichero de disco de una uri del editor, si cae dentro de `trabajo`."""
    if not isinstance(uri, str) or not uri.startswith("file://"):
        return None
    f = os.path.realpath(urllib.parse.unquote(urllib.parse.urlparse(uri).path))
    base = os.path.realpath(trabajo) + os.sep
    if not f.startswith(base) or any(p in FUERA for p in f[len(base):].split(os.sep)):
        return None
    return f


def dentro(trabajo, rel):
    """`trabajo/rel`, si de verdad cae dentro (sin `..`)."""
    f = os.path.realpath(os.path.join(trabajo, rel))
    return f if f.startswith(os.path.realpath(trabajo) + os.sep) else None


# ── lo que pytest dice ─────────────────────────────────────────────────────
def leer_pruebas(texto, pedidos, repo):
    """Las líneas de `ore_informe.py` como resultado: las rutas, del árbol."""
    pruebas = []
    por_fichero = {f: {"ruta": f, "estado": "ok", "salida": ""} for f in pedidos}

    def fichero(f):
        return por_fichero.setdefault(f, {"ruta": f, "estado": "ok", "salida": ""})

    for linea in texto.splitlines():
        try:
            x = json.loads(linea)
        except ValueError:
            continue
        x["fichero"] = "%s/%s" % (repo, x.get("fichero", "")) if x.get("fichero") else repo
        e = x.pop("e", None)
        if e == "prueba":
            pruebas.append({k: v for k, v in x.items() if v is not None})
        elif e == "salida":
            fichero(x["fichero"])["salida"] += x.get("texto", "")
        elif e == "fichero":
            fichero(x["fichero"]).update(estado="error", mensaje=x.get("mensaje", ""))
    for f in por_fichero.values():
        f["salida"] = f["salida"][-20000:]
        if f["estado"] != "error" and any(p["fichero"] == f["ruta"] and p["estado"] == "fallo" for p in pruebas):
            f["estado"] = "fallo"
    return {"ficheros": list(por_fichero.values()), "pruebas": pruebas}


def registro_de(argumentos, terminal, codigo, vencido, tope):
    """El registro como en una terminal: el comando, lo que pytest escribió y cómo acabó."""
    partes = ["$ pytest " + " ".join(shlex.quote(a) for a in argumentos), "", terminal.rstrip()]
    partes += ["", "✖ stopped after %d s (time limit of a test run)" % tope if vencido else "exit code %s" % codigo]
    texto = "\n".join(partes) + "\n"
    if len(texto) > TOPE_REGISTRO:
        texto = "… (%d KB before this are not shown)\n" % ((len(texto) - TOPE_REGISTRO) // 1024) + texto[-TOPE_REGISTRO:]
    return texto


def resumen_de(r):
    n = lambda e: sum(1 for p in r["pruebas"] if p["estado"] == e)  # noqa: E731
    return {"total": len(r["pruebas"]), "ok": n("ok"), "fallo": n("fallo") + n("cancelada"),
            "saltada": n("saltada") + n("pendiente"),
            "ficherosConError": sum(1 for f in r["ficheros"] if f["estado"] == "error")}


def correr_pytest(trabajo, repo, ficheros, nombres=(), rutas=(), tope=None):
    """pytest sobre `ficheros` (del árbol) con `cwd` en el repositorio, en su proceso.

    ⛔ Sin la identidad de la sesión: el entorno del hijo es lo mínimo (sin
      `PUESTO`, sin el token, sin las rutas del almacén). Una prueba unitaria no
      lee datos. Con su tope total y, en Linux, matando su grupo entero.
    """
    tope = tope or int(os.environ.get("ORE_TOPE_PRUEBAS_S", "120"))
    raiz = os.path.join(trabajo, repo)
    rel = [f[len(repo) + 1:] for f in ficheros]
    with tempfile.TemporaryDirectory(prefix="ore-pruebas-") as tmp:
        informe = os.path.join(tmp, "informe.jsonl")
        argumentos = ["-v", "-rA", "--tb=short", "--color=no", "-p", "no:cacheprovider",
                      "--continue-on-collection-errors"] + rel
        entorno = {
            "PATH": os.environ.get("PATH", "/usr/local/bin:/usr/bin:/bin"),
            "HOME": tmp, "LANG": "C.UTF-8", "PYTHONUTF8": "1", "PYTHONDONTWRITEBYTECODE": "1",
            "PYTHONPATH": os.environ.get("ORE_PLUGIN_PYTEST", os.path.join(AQUI, "ore_pytest")),
            "ORE_INFORME": informe,
            "ORE_RUTAS": os.pathsep.join(rutas),
            "ORE_NOMBRES": json.dumps(list(nombres)),
            "ORE_TOPE_PRUEBA": os.environ.get("ORE_TOPE_PRUEBA", "30"),
        }
        hijo = subprocess.Popen([sys.executable, "-m", "pytest", "-p", "ore_informe"] + argumentos,
                                cwd=raiz, env=entorno, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                start_new_session=os.name != "nt")
        vencido = False
        try:
            terminal, _ = hijo.communicate(timeout=tope)
        except subprocess.TimeoutExpired:
            vencido = True
            try:
                os.killpg(hijo.pid, signal.SIGKILL) if os.name != "nt" else hijo.kill()
            except OSError:
                pass
            terminal, _ = hijo.communicate()
        texto = ""
        if os.path.exists(informe):
            with open(informe, encoding="utf-8") as f:
                texto = f.read()
    terminal = terminal.decode("utf-8", "replace")
    r = leer_pruebas(texto, ficheros, repo)
    if vencido:
        r["error"] = "las pruebas tardaron más de %d s: se pararon" % tope
    elif not r["pruebas"] and hijo.returncode not in (0, 1, 5) and not any(f["estado"] == "error" for f in r["ficheros"]):
        r["error"] = terminal.strip()[-2000:] or "pytest salió con %s" % hijo.returncode
    r["registro"] = registro_de(argumentos, terminal, hijo.returncode, vencido, tope)
    return r


# ── lo que pyright dice ────────────────────────────────────────────────────
def leer_pyright(salida, trabajo):
    """`pyright --outputjson` como diagnósticos del árbol (rutas relativas a él)."""
    try:
        j = json.loads(salida)
    except ValueError:
        return {"error": "pyright no contestó JSON: %s" % salida.strip()[-500:]}
    diagnosticos = []
    base = os.path.realpath(trabajo)
    for d in j.get("generalDiagnostics", []):
        if d.get("severity") not in ("error", "warning"):
            continue
        f = os.path.realpath(d.get("file", ""))
        ini = (d.get("range") or {}).get("start") or {}
        diagnosticos.append({
            "fichero": os.path.relpath(f, base).replace(os.sep, "/") if f.startswith(base + os.sep) else d.get("file"),
            "linea": ini.get("line", 0) + 1,
            "columna": ini.get("character", 0) + 1,
            # `pyright:` delante: la consola no los pinta en un `.py` que el
            # servidor de lenguaje ya marca mientras se teclea.
            "codigo": "pyright:" + d["rule"] if d.get("rule") else "pyright",
            "mensaje": d.get("message", ""),
            "severidad": "error" if d["severity"] == "error" else "aviso",
        })
    errores = sum(1 for d in diagnosticos if d["severidad"] == "error")
    return {"diagnosticos": diagnosticos[:500], "errores": errores,
            "ficheros": len({d["fichero"] for d in diagnosticos})}


def correr_pyright(trabajo, repo, rutas, orden=None):
    """pyright sobre el repositorio, con el SDK y la capa a la vista."""
    orden = orden or shlex.split(os.environ.get("ORE_PYRIGHT", "node /opt/ore/pyright/index.js"))
    # ⛔ pyright IGNORA las rutas absolutas de `include` (medido: «Ignoring path
    #   … because it is not relative», 0 ficheros): su configuración va junto
    #   al árbol, con el repositorio relativo a ella, y se borra al acabar.
    fd, config = tempfile.mkstemp(prefix=".ore-pyright-", suffix=".json", dir=trabajo)
    os.close(fd)
    try:
        with open(config, "w", encoding="utf-8") as f:
            json.dump({
                "include": [repo],
                "exclude": ["**/__pycache__", "**/.venv", "**/node_modules"],
                "extraPaths": [r for r in rutas if os.path.isdir(r)],
                "pythonVersion": "%d.%d" % sys.version_info[:2],
                "typeCheckingMode": "basic",
            }, f)
        try:
            r = subprocess.run(orden + ["--outputjson", "--pythonpath", sys.executable, "-p", config],
                               cwd=trabajo, capture_output=True, timeout=120)
        except FileNotFoundError:
            return {"error": "no hay pyright en esta sesión (%s)" % " ".join(orden)}
        except subprocess.TimeoutExpired:
            return {"error": "pyright tardó más de 120 s"}
    finally:
        os.remove(config)
    return leer_pyright(r.stdout.decode("utf-8", "replace"), trabajo)


# ── el repositorio de la sesión ────────────────────────────────────────────
class Repositorio:
    """El repositorio del puesto en `trabajo`, y lo que se le pide."""

    def __init__(self, puesto, testigo, trabajo=None):
        self.p = puesto
        self.testigo = testigo
        self.trabajo = trabajo or os.environ.get("TRABAJO_DIR", "/trabajo")
        # Probar y comprobar traen el repositorio al MISMO disco: una tras otra.
        self.candado = threading.Lock()
        sdk = os.environ.get("ORE_SDK_DIR", AQUI)
        capa = os.environ.get("ORE_CAPA_DIR", "/capa")
        self.rutas = [sdk, capa, os.path.join(capa, ".dev")]

    def _pedir(self, metodo, ruta, cabeceras=None, plazo=30):
        self.p._cabeceras = self.testigo.cabeceras()
        return self.p.pedir(metodo, ruta, cabeceras=cabeceras, plazo=plazo)

    def materializar(self):
        """El repositorio de la rama del puesto, en disco. Devuelve su ruta o None."""
        t0 = time.time()
        c, ficha = self._pedir("GET", "/puestos/%s" % self.p.id)
        repo = (ficha or {}).get("repositorio") if c == 200 else None
        if not repo:
            log("sin repositorio: pyright sólo verá lo que el editor abra")
            return None
        rama = {"x-ore-rama": ficha["rama"]} if ficha.get("rama") else {}
        ci, indice = self._pedir("GET", "/arbol", cabeceras=dict(rama, **{"x-ore-raiz": repo}), plazo=60)
        if ci != 200:
            log("el índice de %s contestó %s: sin repositorio en disco" % (repo, ci))
            return None
        todos = [f for f in (indice or {}).get("ficheros", [])
                 if DE_REPOSITORIO.search(f.get("ruta", "")) and (f.get("bytes") or 0) <= BYTES_MAXIMOS
                 and not f["ruta"].endswith("pylock.toml")]
        lista = todos[:FICHEROS_MAXIMOS]
        if len(todos) > len(lista):
            log("⚠️ %d ficheros: se traen %d" % (len(todos), len(lista)))
        n = 0
        for f in lista:
            destino = dentro(self.trabajo, f["ruta"])
            if not destino:
                continue
            ruta = "/".join(urllib.parse.quote(p) for p in f["ruta"].split("/"))
            cf, r = self._pedir("GET", "/arbol/%s" % ruta, cabeceras=rama)
            if cf != 200 or not isinstance((r or {}).get("texto"), str):
                continue
            os.makedirs(os.path.dirname(destino), exist_ok=True)
            with open(destino, "w", encoding="utf-8", newline="") as fh:
                fh.write(r["texto"])
            n += 1
        # Lo que la rama ya no tiene, tampoco el disco: una prueba borrada no corre.
        en_la_rama = {f["ruta"] for f in lista}
        fuera = 0
        for rel in ficheros_que(lambda x: bool(DE_REPOSITORIO.search(x)), os.path.join(self.trabajo, repo)):
            if "%s/%s" % (repo, rel) not in en_la_rama:
                os.remove(os.path.join(self.trabajo, repo, rel))
                fuera += 1
        log("%s%s en disco · %d fichero(s)%s en %d ms" % (
            repo, " (%s)" % ficha["rama"] if ficha.get("rama") else "", n,
            ", %d retirado(s)" % fuera if fuera else "", (time.time() - t0) * 1000))
        return repo

    def espejo(self, m):
        """Lo que el editor abre o cambia (el texto entero), también en disco."""
        if m.get("method") == "textDocument/didOpen":
            td = (m.get("params") or {}).get("textDocument") or {}
            uri, texto = td.get("uri"), td.get("text")
        elif m.get("method") == "textDocument/didChange":
            p = m.get("params") or {}
            uri = (p.get("textDocument") or {}).get("uri")
            cambios = p.get("contentChanges") or []
            texto = cambios[-1].get("text") if cambios and all("range" not in c for c in cambios) else None
        else:
            return
        f = en_disco(uri, self.trabajo)
        if f and isinstance(texto, str):
            try:
                os.makedirs(os.path.dirname(f), exist_ok=True)
                with open(f, "w", encoding="utf-8", newline="") as fh:
                    fh.write(texto)
            except OSError as e:
                log("no pude reflejar %s (%s)" % (f, e))

    def probar(self, q):
        t0 = time.time()
        try:
            with self.candado:
                repo = self.materializar()
                if not repo:
                    r = {"error": "este puesto no tiene repositorio: no hay pruebas que correr"}
                else:
                    # Los borradores de la consola, encima de lo guardado (sólo los del repositorio).
                    for b in q.get("borradores") or []:
                        ruta, texto = (b or {}).get("ruta"), (b or {}).get("texto")
                        f = dentro(self.trabajo, ruta) if isinstance(ruta, str) and ruta.startswith(repo + "/") else None
                        if f and isinstance(texto, str):
                            os.makedirs(os.path.dirname(f), exist_ok=True)
                            with open(f, "w", encoding="utf-8", newline="") as fh:
                                fh.write(texto)
                    todos = ["%s/%s" % (repo, f) for f in ficheros_que(es_de_prueba, os.path.join(self.trabajo, repo))]
                    pedidos = [f for f in todos if f in (q.get("ficheros") or [])] if q.get("ficheros") else todos
                    r = {"repositorio": repo, "todos": todos}
                    if pedidos:
                        r.update(correr_pytest(self.trabajo, repo, pedidos, q.get("nombres") or [], self.rutas))
                    else:
                        r.update(ficheros=[], pruebas=[])
        except Exception as e:  # noqa: BLE001 — el editor no se queda colgado
            r = {"error": str(e)}
        r["ms"] = int((time.time() - t0) * 1000)
        if "pruebas" in r:
            r["resumen"] = resumen_de(r)
        log("probar · %s · %d ms" % (r.get("error") or "%(ok)d bien, %(fallo)d mal, %(saltada)d saltada(s)" % r["resumen"], r["ms"]))
        return r

    def comprobar(self):
        t0 = time.time()
        try:
            with self.candado:
                repo = self.materializar()
                if not repo:
                    r = {"error": "este puesto no tiene repositorio: no hay proyecto que comprobar"}
                else:
                    r = dict(repositorio=repo, **correr_pyright(self.trabajo, repo, self.rutas))
        except Exception as e:  # noqa: BLE001
            r = {"error": str(e)}
        r["ms"] = int((time.time() - t0) * 1000)
        log("comprobar · %s · %d ms" % (r.get("error") or "%d error(es) en %d fichero(s)" % (r["errores"], r["ficheros"]), r["ms"]))
        return r
