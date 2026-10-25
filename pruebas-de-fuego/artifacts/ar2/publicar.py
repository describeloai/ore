"""ORE 0074 AR2·0 · publicar: el orden del multipart de `uv publish` y `twine`, el 409 al republicar
y el nombre de fichero que da `uv build` con `version = "1.0"`.

Uso: python publicar.py <uv> <twine> <raiz>   (con `indices.py <raiz>` corriendo)
`uv build` necesita su backend (`uv_build`): es lo único que baja de internet (PyPI real, leer).
"""
import json, os, shutil, subprocess, sys

UV, TWINE, RAIZ = sys.argv[1], sys.argv[2], os.path.abspath(sys.argv[3])
ORE, CLAVE = "http://127.0.0.1:8091", "ore_art_juguete"
DIST = os.path.join(RAIZ, "dist2")


def correr(a, cwd=None):
    r = subprocess.run(a, cwd=cwd, capture_output=True, text=True, encoding="utf-8", errors="replace",
                       env={k: v for k, v in os.environ.items() if not k.startswith("UV_")})
    return r.returncode, (r.stdout + r.stderr).strip()


def posts_desde(n):
    ls = open(os.path.join(RAIZ, "registro.jsonl"), encoding="utf-8").readlines()[n:]
    return [json.loads(l) for l in ls if json.loads(l)["metodo"] == "POST"]


def n():
    return sum(1 for _ in open(os.path.join(RAIZ, "registro.jsonl"), encoding="utf-8"))


shutil.rmtree(DIST, ignore_errors=True)
# Un proyecto de verdad, construido con `uv build` (ruedas y sdist con el nombre que da la herramienta).
proy = os.path.join(RAIZ, "proyecto-banco")
shutil.rmtree(proy, ignore_errors=True)
os.makedirs(os.path.join(proy, "src", "banco_riesgo"))
open(os.path.join(proy, "src", "banco_riesgo", "__init__.py"), "w").write("X = 1\n")
open(os.path.join(proy, "pyproject.toml"), "w").write("\n".join([
    "[project]", 'name = "Banco_Riesgo"', 'version = "1.0"', 'requires-python = ">=3.10"', "",
    "[build-system]", 'requires = ["uv_build>=0.9,<0.13"]', 'build-backend = "uv_build"', ""]))
c, o = correr([UV, "build", "--out-dir", DIST], proy)
print("uv build:", c, o[-300:])
print("ficheros:", sorted(os.listdir(DIST)) if os.path.isdir(DIST) else None)
ficheros = [os.path.join(DIST, f) for f in sorted(os.listdir(DIST))] if os.path.isdir(DIST) else []

r = {}
k = n()
r["uv_publish"] = correr([UV, "publish", "--publish-url", f"{ORE}/upload/", "--username", "__token__", "--password", CLAVE] + ficheros)
r["uv_publish_posts"] = posts_desde(k)
k = n()
r["uv_republicar"] = correr([UV, "publish", "--publish-url", f"{ORE}/upload/", "--username", "__token__", "--password", CLAVE] + ficheros)
r["uv_republicar_posts"] = posts_desde(k)
k = n()
r["twine_republicar"] = correr([TWINE, "upload", "--non-interactive", "--disable-progress-bar", "--repository-url",
                                f"{ORE}/upload/", "-u", "__token__", "-p", CLAVE] + ficheros)
r["twine_republicar_posts"] = posts_desde(k)
open(os.path.join(RAIZ, "publicar.json"), "w", encoding="utf-8").write(json.dumps(r, ensure_ascii=False, indent=1))
for clave in ("uv_publish", "uv_republicar", "twine_republicar"):
    print(f"· {clave}: código {r[clave][0]} · {r[clave][1][-300:]}")
    for p in r[clave + "_posts"]:
        print("   POST", p["estado"], p["usuario"], p["fichero"], p["nombre"], p["version"], p["filetype"],
              p["requires_python"], "chunked" if p["chunked"] else "", p["expect"] or "", p["agente"])
        print("        orden:", p["orden"])
