# RESOLVER LA CAPA DE PYTHON (ORE 0050 P2) — dentro de `capa-python:1`.
#
# El gemelo de `puesto/node/capa/capa.mjs`, para uv:
#
#   declarar <árbol> <alcance> <trabajo>   los pyproject.toml del alcance → deps.txt · digest.txt
#   proyecto <trabajo> <provisto>          el proyecto que uv resuelve: lo declarado, con lo que
#                                          la sesión trae como RESTRICCIÓN → proyecto/pyproject.toml
#   fuera    <trabajo> <pylock> <provisto> los nombres que una caja no lleva (`--no-emit-package`)
#   informe  <trabajo> <estado>            lo resuelto → informe.json (lock, sumas, avisos) y, si
#                                          está lista, lock-del-repositorio.toml (el `pylock.toml`)
#
# ⚠️ LA DECLARACIÓN Y EL DIGEST SON LOS DE `ore-serve` (`entorno.rs`,
#   `dependencias_de` y `digest_de`/`semilla_python`), byte a byte: si
#   difieren, el Job resuelve una capa con otro nombre del que la sesión
#   espera. Lo mismo que allí: `[project].dependencies` y, como `dev:`, el
#   grupo `dev` de `[dependency-groups]` (sólo sus cadenas); la unión ordenada
#   por bytes y sin repetidos; y el digest es `capa-` + 12 hex del sha256 del
#   intérprete (`cp314`) + `\n` + las líneas.
#
# ⭐ LO QUE LA SESIÓN TRAE ES UNA RESTRICCIÓN, NO UN AVISO (P2). En Node, si
#   se pide otra versión de lo que trae la sesión, gana la de la sesión y el
#   informe lo dice. En Python dos `numpy` no conviven —un ABI que no casa no
#   da excepción, mata al intérprete—, así que se resuelve CON las versiones
#   de la sesión (`constraint-dependencies`): lo compatible se resuelve, y lo
#   que no, es un error que lo dice. Y el lock dice la verdad: es lo que la
#   sesión importará.
import hashlib
import json
import os
import re
import sys
import time
import tomllib

DEV = "dev:"
# El SDK no es un paquete del registro: lo pone la sesión (`/opt/ore/ore`).
# Declararlo no puede bajar a PyPI otro `ore` que no es el nuestro.
RESERVADAS = {"ore"}


def normal(n):
    """PEP 503: `Google_Cloud.Storage` → `google-cloud-storage`."""
    return re.sub(r"[-_.]+", "-", n).strip().lower()


def nombre_de(req):
    """El nombre de un requisito de PEP 508 (`polars[pyarrow]>=1` → `polars`)."""
    return re.split(r"[<>=!~;\[ (@]", req.strip(), maxsplit=1)[0]


def por_bytes(xs):
    """El orden de `Vec<String>::sort` de Rust: por bytes."""
    return sorted(xs, key=lambda s: s.encode())


def declaradas(texto):
    try:
        t = tomllib.loads(texto)
    except tomllib.TOMLDecodeError:
        return []
    fuera = []
    deps = t.get("project", {}).get("dependencies", [])
    dev = t.get("dependency-groups", {}).get("dev", [])
    for lista, prefijo in ((deps, ""), (dev, DEV)):
        if not isinstance(lista, list):
            continue
        for d in lista:
            # `{ include-group = … }` no se honra: sólo cadenas.
            if isinstance(d, str) and d.strip():
                fuera.append(prefijo + d.strip())
    return fuera


def ficheros(arbol, alcance):
    """Los pyproject.toml de un alcance, como `declaracion_en`."""
    f = [os.path.join(arbol, "pyproject.toml")]
    a = (alcance or "").strip().strip("/")
    if not a:
        p = os.path.join(arbol, "packages")
        if os.path.isdir(p):
            f += [os.path.join(p, e, "pyproject.toml") for e in os.listdir(p)]
    else:
        partes = a.split("/")
        if len(partes) >= 2 and partes[0] == "packages":
            acc = os.path.join(arbol, "packages", partes[1])
            f.append(os.path.join(acc, "pyproject.toml"))
            for p in partes[2:]:
                acc = os.path.join(acc, p)
                f.append(os.path.join(acc, "pyproject.toml"))
    return f


def abi():
    return "cp%d%d" % sys.version_info[:2]


def digest_de(deps, abi_):
    if not deps:
        return ""
    semilla = "\n".join(deps) if abi_ == "cp312" else abi_ + "\n" + "\n".join(deps)
    return "capa-" + hashlib.sha256(semilla.encode()).hexdigest()[:12]


def declarar(arbol, alcance, trabajo):
    deps = set()
    for f in ficheros(arbol, alcance):
        if os.path.isfile(f):
            with open(f, encoding="utf-8") as fh:
                deps.update(declaradas(fh.read()))
    deps = por_bytes(deps)
    digest = digest_de(deps, abi())
    escribir(trabajo, "deps.txt", "\n".join(deps) + "\n")
    escribir(trabajo, "digest.txt", digest)
    print("### alcance:", alcance or "(la celda)")
    print("### declarado:", ", ".join(deps) or "(nada)", "→", digest or "(sin capa)")


def provisto(f):
    """`nombre==versión` por línea (`puesto/python/provisto.txt`)."""
    trae = {}
    for l in open(f, encoding="utf-8"):
        l = l.strip()
        if l and not l.startswith("#") and "==" in l:
            n, v = l.split("==", 1)
            trae[normal(n)] = (n.strip(), v.strip())
    return trae


def toml_lista(xs):
    return "[" + ", ".join(json.dumps(x) for x in xs) + "]"


def proyecto(trabajo, fichero_provisto):
    deps = [l for l in leer(trabajo, "deps.txt").split("\n") if l]
    trae = provisto(fichero_provisto)
    run, dev, avisos = [], [], []
    for d in deps:
        es_dev = d.startswith(DEV)
        req = d[len(DEV):] if es_dev else d
        n = normal(nombre_de(req))
        if n in RESERVADAS:
            avisos.append("%s es el SDK, y lo pone la sesión: no se instala del registro" % nombre_de(req))
            continue
        # Una versión exacta distinta de la de la sesión no puede resolverse:
        # se dice en su idioma antes de que uv lo diga en el suyo.
        m = re.fullmatch(r"\s*==\s*([^\s;,]+)\s*", req[len(nombre_de(req)):].split(";")[0])
        if n in trae and m and m.group(1) != trae[n][1]:
            avisos.append(
                "pediste %s %s, y esta sesión trae la %s: en Python no conviven dos versiones del mismo "
                "paquete; quita la versión o pide una compatible" % (trae[n][0], m.group(1), trae[n][1]))
        (dev if es_dev else run).append(req)
    os.makedirs(os.path.join(trabajo, "proyecto"), exist_ok=True)
    restricciones = ["%s==%s" % v for v in trae.values()]
    escribir(os.path.join(trabajo, "proyecto"), "pyproject.toml", "\n".join([
        "# Lo que el repositorio declara, para que uv lo resuelva (ORE 0050 P2).",
        "[project]",
        'name = "capa"',
        'version = "0"',
        'requires-python = "==%d.%d.*"' % sys.version_info[:2],
        "dependencies = " + toml_lista(run),
        "",
        "[dependency-groups]",
        "dev = " + toml_lista(dev),
        "",
        "[tool.uv]",
        "package = false",
        # Sólo ruedas: ningún paquete compila ni ejecuta código al instalarse.
        "no-build = true",
        "environments = [\"sys_platform == 'linux' and platform_machine == 'x86_64'\"]",
        # Lo que la sesión trae, a su versión: una restricción, no un aviso.
        "constraint-dependencies = " + toml_lista(restricciones),
        "",
    ]))
    escribir(trabajo, "avisos.txt", "\n".join(avisos) + ("\n" if avisos else ""))
    print("### a uv: %d paquete(s)%s%s" % (
        len(run), " + %d de desarrollo" % len(dev) if dev else "",
        " · %d aviso(s)" % len(avisos) if avisos else ""))


def paquetes(pylock):
    """`[(nombre, versión)]` de un `pylock.toml`; vacío si no existe."""
    if not os.path.isfile(pylock):
        return []
    with open(pylock, "rb") as fh:
        return [(p["name"], p["version"]) for p in tomllib.load(fh).get("packages", []) if p.get("version")]


def fuera(trabajo, pylock, fichero_provisto):
    """Lo que una caja no lleva: lo de la sesión y, si se da, lo de otra caja."""
    nombres = {normal(n) for n in provisto(fichero_provisto)} | RESERVADAS
    nombres |= {normal(n) for n, _ in paquetes(pylock)} if pylock != "-" else set()
    print(" ".join("--no-emit-package %s" % n for n in sorted(nombres)))


def suma_de(f):
    if not os.path.isfile(f):
        return "", 0
    b = open(f, "rb").read()
    return hashlib.sha256(b).hexdigest(), -(-len(b) // 1048576)


def informe(trabajo, estado):
    deps = [l for l in leer(trabajo, "deps.txt").split("\n") if l]
    digest = leer(trabajo, "digest.txt").strip()
    avisos = [l for l in leer(trabajo, "avisos.txt").split("\n") if l]
    trae = provisto(os.environ.get("PROVISTO", "/opt/ore/provisto.txt"))
    proy = os.path.join(trabajo, "proyecto")
    # El lock: lo que la capa trae —lo de la sesión no, como en Node—, y lo de
    # desarrollo con `dev:` delante (lo que la resolución entera tiene y la de
    # ejecución no).
    todo = paquetes(os.path.join(proy, "pylock.toml"))
    corre = {normal(n) for n, _ in paquetes(os.path.join(proy, "pylock.run.toml"))}
    lock = por_bytes(
        ("" if normal(n) in corre else DEV) + "%s==%s" % (n, v)
        for n, v in todo if normal(n) not in trae and normal(n) not in RESERVADAS)
    suma, mb = suma_de(os.path.join(trabajo, "capa.tgz"))
    suma_dev, mb_dev = suma_de(os.path.join(trabajo, "dev.tgz"))
    tope = int(os.environ.get("TOPE_MB", "512"))
    error = ""
    if estado == "error":
        error = ultimas(os.path.join(trabajo, "uv.log"), 800)
    elif mb > tope:
        estado, error = "error", "la capa pesa %d MB y el tope es %d MB: un puesto que tarda dos minutos en arrancar no es un puesto" % (mb, tope)
    elif mb_dev > tope:
        estado, error = "error", "lo de desarrollo pesa %d MB y el tope es %d MB" % (mb_dev, tope)
    # El lock para el repositorio: el `pylock.toml` ENTERO —lo de la sesión
    # incluido, fijado a su versión: es la restricción con la que se resolvió—,
    # con nuestra cabecera en vez del comando de uv. Sólo de una capa lista.
    del_repo = os.path.join(trabajo, "lock-del-repositorio.toml")
    if estado == "lista" and todo:
        cuerpo = [l for l in leer(proy, "pylock.toml").split("\n") if not l.startswith("#")]
        escribir(trabajo, "lock-del-repositorio.toml", "\n".join([
            "# What this repository's pyproject.toml resolved to (PEP 751), written by the",
            "# platform on every change: don't edit it. Packages the session already",
            "# provides appear pinned to its version: they are part of the resolution.",
        ] + cuerpo).rstrip("\n") + "\n")
    elif os.path.exists(del_repo):
        os.remove(del_repo)
    j = {"estado": estado, "digest": digest, "declarado": deps}
    if suma:
        j.update(caja="capa.tgz", suma=suma)
    if suma_dev:
        j.update(cajaDev="dev.tgz", sumaDev=suma_dev, mbDev=str(mb_dev))
    j.update(lock=lock, mb=str(mb), avisos=avisos,
             cuando=time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
             entorno="puesto-python:1", python="%d.%d" % sys.version_info[:2])
    if error:
        j["error"] = error
    escribir(trabajo, "informe.json", json.dumps(j, indent=1, ensure_ascii=False) + "\n")
    print("### informe %s · %d paquete(s) · %d MB%s%s" % (
        estado, len(lock), mb, " + %d MB de desarrollo" % mb_dev if suma_dev else "",
        " · %d aviso(s)" % len(avisos) if avisos else ""))
    for a in avisos:
        print("    ⚠️ " + a)


def leer(d, f):
    p = os.path.join(d, f)
    return open(p, encoding="utf-8").read() if os.path.isfile(p) else ""


def escribir(d, f, t):
    with open(os.path.join(d, f), "w", encoding="utf-8") as fh:
        fh.write(t)


def ultimas(f, n):
    t = open(f, encoding="utf-8", errors="replace").read() if os.path.isfile(f) else ""
    return "…" + t[-n:] if len(t) > n else t


if __name__ == "__main__":
    orden, args = (sys.argv[1] if len(sys.argv) > 1 else ""), sys.argv[2:]
    if orden == "declarar":
        declarar(*args)
    elif orden == "proyecto":
        proyecto(*args)
    elif orden == "fuera":
        fuera(*args)
    elif orden == "informe":
        informe(*args)
    else:
        sys.exit("capa.py declarar|proyecto|fuera|informe …")
