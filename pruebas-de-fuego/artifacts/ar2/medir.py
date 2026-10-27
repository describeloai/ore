"""ORE 0074 AR2·0 · el previo de uv: lo que la capa de Python hará con un índice de ORE delante de PyPI.

Uso: python medir.py <uv> <twine> <raiz>   (con `indices.py <raiz>` ya corriendo)

Ruedas y sdists hechos a mano (sin backend de build ni red); el «PyPI» es el falso de `indices.py`.
Cada caso resuelve un proyecto como el que escribe `puesto/python/capa/capa.py proyecto` (`uv lock`
y `uv export --format pylock.toml`) y apunta en `<raiz>/casos.jsonl` qué se resolvió, de dónde, el
error si lo hubo y las peticiones que hizo a cada índice.
"""
import base64, hashlib, io, json, os, shutil, subprocess, sys, tarfile, time, zipfile

UV, TWINE, RAIZ = sys.argv[1], sys.argv[2], os.path.abspath(sys.argv[3])
CLAVE = "ore_art_juguete"
ORE, PYPI, MUERTO = "http://127.0.0.1:8091", "http://127.0.0.1:8092", "http://127.0.0.1:8093"
PY = "%d.%d" % sys.version_info[:2]
CASOS = os.path.join(RAIZ, "casos.jsonl")
DIST = os.path.join(RAIZ, "dist")


def rueda(nombre, version, deps=(), requiere=None):
    """Una rueda py3-none-any mínima y válida."""
    dist = nombre.replace("-", "_")
    di = f"{dist}-{version}.dist-info"
    meta = [f"Metadata-Version: 2.1", f"Name: {nombre}", f"Version: {version}"]
    meta += [f"Requires-Dist: {d}" for d in deps]
    if requiere:
        meta.append(f"Requires-Python: {requiere}")
    ficheros = {
        f"{dist}/__init__.py": f"VERSION = {version!r}\n",
        f"{di}/METADATA": "\n".join(meta) + "\n",
        f"{di}/WHEEL": "Wheel-Version: 1.0\nGenerator: ore-ar2\nRoot-Is-Purelib: true\nTag: py3-none-any\n",
    }
    record = []
    for n, t in ficheros.items():
        h = base64.urlsafe_b64encode(hashlib.sha256(t.encode()).digest()).rstrip(b"=").decode()
        record.append(f"{n},sha256={h},{len(t.encode())}")
    record.append(f"{di}/RECORD,,")
    ficheros[f"{di}/RECORD"] = "\n".join(record) + "\n"
    os.makedirs(DIST, exist_ok=True)
    p = os.path.join(DIST, f"{dist}-{version}-py3-none-any.whl")
    with zipfile.ZipFile(p, "w") as z:
        for n, t in ficheros.items():
            z.writestr(n, t)
    return p


def sdist(nombre, version):
    dist = nombre.replace("-", "_")
    p = os.path.join(DIST, f"{dist}-{version}.tar.gz")
    with tarfile.open(p, "w:gz") as t:
        pk = f"Metadata-Version: 2.1\nName: {nombre}\nVersion: {version}\n".encode()
        i = tarfile.TarInfo(f"{dist}-{version}/PKG-INFO")
        i.size = len(pk)
        t.addfile(i, io.BytesIO(pk))
    return p


def sembrar_pypi(p):
    """Directo al disco del «PyPI» falso: no es lo que se mide."""
    nombre = os.path.basename(p).split("-")[0].replace("_", "-").lower()
    d = os.path.join(RAIZ, "pypi", nombre)
    os.makedirs(d, exist_ok=True)
    shutil.copy(p, d)


def modo(m):
    open(os.path.join(RAIZ, "modo-ore.txt"), "w").write(m)


def registro_desde(n):
    with open(os.path.join(RAIZ, "registro.jsonl"), encoding="utf-8") as f:
        return [json.loads(l) for l in f.readlines()[n:]]


def lineas():
    try:
        return sum(1 for _ in open(os.path.join(RAIZ, "registro.jsonl"), encoding="utf-8"))
    except OSError:
        return 0


def correr(args, env=None, cwd=None):
    t0 = time.monotonic()
    r = subprocess.run(args, env=env, cwd=cwd, capture_output=True, text=True, encoding="utf-8", errors="replace")
    return r.returncode, (r.stdout + r.stderr).strip(), round(time.monotonic() - t0, 2)


def entorno(ore=ORE, credencial=True):
    env = dict(os.environ)
    for k in list(env):
        if k.startswith("UV_"):
            del env[k]
    env.update(UV_CACHE_DIR=os.path.join(RAIZ, "cache-%d" % time.monotonic_ns()), UV_PYTHON_DOWNLOADS="never",
               UV_NO_PROGRESS="1", UV_PYTHON=sys.executable,
               UV_INDEX=f"ore={ore}/simple/", UV_DEFAULT_INDEX=f"{PYPI}/simple/")
    if credencial:
        env.update(UV_INDEX_ORE_USERNAME="capa", UV_INDEX_ORE_PASSWORD=CLAVE)
    return env


def caso(nombre, deps, **k):
    d = os.path.join(RAIZ, "proyectos", nombre)
    shutil.rmtree(d, ignore_errors=True)
    os.makedirs(d)
    open(os.path.join(d, "pyproject.toml"), "w").write("\n".join([
        "[project]", 'name = "capa"', 'version = "0"', f'requires-python = "=={PY}.*"',
        "dependencies = " + json.dumps(list(deps)), "", "[tool.uv]", "package = false", "no-build = true", ""]))
    n0 = lineas()
    env = entorno(**k)
    c, salida, s = correr([UV, "lock"], env, d)
    resuelto, lock_token, lock_ore = {}, None, None
    if c == 0:
        correr([UV, "export", "-q", "--format", "pylock.toml", "--no-emit-project", "-o", "pylock.toml"], env, d)
        lock = open(os.path.join(d, "uv.lock"), encoding="utf-8").read()
        pylock = open(os.path.join(d, "pylock.toml"), encoding="utf-8").read() if os.path.exists(os.path.join(d, "pylock.toml")) else ""
        lock_token = CLAVE in lock or CLAVE in pylock or "capa:" in lock or "capa:" in pylock
        lock_ore = ("8091" in lock, "8091" in pylock)
        nom = None
        for l in lock.splitlines():
            if l.startswith("name = "):
                nom = l.split('"')[1]
            elif l.startswith("version = ") and nom and nom != "capa":
                resuelto[nom] = l.split('"')[1]
            elif l.startswith("source = ") and nom in resuelto:
                resuelto[nom] += " ← " + ("ore" if "8091" in l else "pypi" if "8092" in l else l)
                nom = None
    peticiones = [(p["indice"], p["metodo"], p["ruta"], p["estado"], p.get("usuario"), p.get("range"), (p.get("accept") or "")[:60])
                  for p in registro_desde(n0)]
    r = {"caso": nombre, "deps": deps, "codigo": c, "segundos": s, "resuelto": resuelto,
         "token_en_el_lock": lock_token, "url_de_ore_en_uv_lock_y_pylock": lock_ore,
         "error": None if c == 0 else salida[-500:], "peticiones": peticiones}
    with open(CASOS, "a", encoding="utf-8") as f:
        f.write(json.dumps(r, ensure_ascii=False) + "\n")
    print(f"· {nombre}: código {c} · {resuelto or salida[-160:]}")
    return r


def main():
    for f in ("casos.jsonl", "registro.jsonl"):
        if os.path.exists(os.path.join(RAIZ, f)):
            os.remove(os.path.join(RAIZ, f))
    modo("normal")
    # ── el «PyPI» falso ──
    sembrar_pypi(rueda("banco-riesgo", "9.9.9"))                       # el atacante
    sembrar_pypi(rueda("app-dep", "1.0.0", ["banco-riesgo>=1"]))        # una pública que pide la interna
    sembrar_pypi(rueda("utilidad", "2.0.0"))                           # sólo pública
    # ── publicar en «ORE»: el orden del multipart, uv y twine ──
    w1 = rueda("Banco_Riesgo", "1.0.0", requiere=">=3.10")
    s1 = sdist("Banco_Riesgo", "1.0.0")
    n0 = lineas()
    c, o, _ = correr([UV, "publish", "--publish-url", f"{ORE}/upload/", "--username", "__token__", "--password", CLAVE, w1, s1],
                     entorno())
    print("· uv publish:", c, o[-200:])
    w2 = rueda("banco.riesgo", "1.1.0", requiere=">=3.10")
    c2, o2, _ = correr([TWINE, "upload", "--non-interactive", "--disable-progress-bar", "--repository-url", f"{ORE}/upload/",
                        "-u", "__token__", "-p", CLAVE, w2], dict(os.environ))
    print("· twine upload:", c2, o2[-200:])
    c3, o3, _ = correr([UV, "publish", "--publish-url", f"{ORE}/upload/", "--username", "__token__", "--password", CLAVE, w1],
                       entorno())
    print("· uv publish otra vez:", c3, o3[-300:])
    pubs = [p for p in registro_desde(n0) if p["metodo"] == "POST"]
    with open(CASOS, "a", encoding="utf-8") as f:
        f.write(json.dumps({"caso": "publicar", "uv": [c, o[-300:]], "twine": [c2, o2[-300:]], "republicar": [c3, o3[-400:]],
                            "posts": pubs}, ensure_ascii=False) + "\n")
    rueda("modelo-credito", "1.0.0", ["banco-riesgo>=1"])
    sembrar = os.path.join(RAIZ, "ore", "modelo-credito")
    os.makedirs(sembrar, exist_ok=True)
    shutil.copy(os.path.join(DIST, "modelo_credito-1.0.0-py3-none-any.whl"), sembrar)
    # ── resolver ──
    caso("A-directa", ["banco-riesgo>=1"])
    caso("B-transitiva-desde-una-publica", ["app-dep"])
    caso("B2-transitiva-desde-una-de-ore", ["modelo-credito"])
    caso("C-version-que-ore-no-tiene", ["banco-riesgo==9.9.9"])
    caso("D-solo-publica-404-en-ore", ["utilidad"])
    caso("I-sin-credencial", ["banco-riesgo>=1"], credencial=False)
    modo("401"); caso("E-ore-401", ["banco-riesgo>=1"])
    modo("403"); caso("F-ore-403", ["banco-riesgo>=1"])
    modo("500"); caso("G-ore-500", ["banco-riesgo>=1"])
    modo("normal")
    caso("H-ore-no-contesta", ["banco-riesgo>=1"], ore=MUERTO)
    caso("H2-ore-no-contesta-y-solo-publica", ["utilidad"], ore=MUERTO)
    modo("403"); caso("F2-ore-403-y-solo-publica", ["utilidad"])
    modo("normal")


main()
