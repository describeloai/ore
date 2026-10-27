#!/usr/bin/env python3
"""LA HUELLA DE UNA PRUEBA LARGA: 16 hex de todo lo que decide lo que un job de pruebas de `ci.yml`
prueba. Si un job ya salió verde con esta huella, lo que prueba no ha cambiado y no hace falta
esperarlo para desplegar (`cambios` y `puerta` en `ci.yml`). Si cambia, se corre y se espera.

    python3 ci/huella-de-las-pruebas.py conectores hbase-kerberos   # «job clave», uno por línea
    python3 ci/huella-de-las-pruebas.py --github conectores         # «conectores=prueba-…», para $GITHUB_OUTPUT
    python3 ci/huella-de-las-pruebas.py --explicar conectores       # de qué sale

Lo que entra en la de un job, leído del propio job (nada que mantener a mano):
  · su bloque de `ci.yml`, sin las líneas que sólo son comentario;
  · `Cargo.toml`, `Cargo.lock` y `rust-toolchain.toml` de la raíz;
  · cada paquete que el job nombra (`-p <paquete>`, `--features <paquete>/…`, también dentro de
    los guiones que nombra) y cada paquete del workspace del que dependen, CON `[dev-dependencies]`
    y `[build-dependencies]`: la carpeta entera, `tests/` incluido, sin `target/` ni los `.md`;
  · lo que el código de esos paquetes incluye de fuera de su carpeta (`include_str!`…);
  · cada ruta de `pruebas-de-fuego/`, `ci/`, `crates/` o `puesto/` que el job o sus guiones
    nombran. De `pruebas-de-fuego/<x>/…`, la carpeta `<x>` entera: los guiones de ahí se llaman
    entre sí con `$AQUI/..`, y eso no se lee de un texto.

Por el CONTENIDO de los ficheros y no por un diff de git: un commit que sólo toca `ore-serve`
detrás de uno que rompió HBase lleva la huella del que lo rompió, y esa no tiene ninguna corrida
verde. Con un diff contra el commit de antes se desplegaría el HBase roto.
"""
import hashlib
import importlib.util
import os
import re
import sys
import tomllib

AQUI = os.path.dirname(os.path.abspath(__file__))
RAIZ = os.environ.get("ORE_RAIZ") or os.path.normpath(os.path.join(AQUI, ".."))
COMUNES = ["Cargo.toml", "Cargo.lock", "rust-toolchain.toml"]
SIN = {"target", "node_modules", "__pycache__"}
PAQUETE = re.compile(r"(?:^|\s)-p\s+([A-Za-z][A-Za-z0-9_-]*)|--features[\s=]+([A-Za-z0-9_-]+)/")
RUTA = re.compile(r"(?<![\w./-])((?:pruebas-de-fuego|ci|crates|puesto)/[A-Za-z0-9_./-]*[A-Za-z0-9_-])")
GUION = (".sh", ".py", ".rb")

# Lo que lee el código lo sabe ya la huella de los binarios: el mismo criterio para los dos.
_spec = importlib.util.spec_from_file_location("binarios", os.path.join(AQUI, "huella-de-los-binarios.py"))
binarios = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(binarios)
binarios.RAIZ = RAIZ


def sha(ruta):
    return hashlib.sha256(open(ruta, "rb").read()).hexdigest()


def rel(ruta):
    return os.path.relpath(ruta, RAIZ).replace(os.sep, "/")


def bloque(job):
    """El bloque del job en `ci.yml`, de `  <job>:` al siguiente job, sin las líneas de comentario."""
    lineas = open(os.path.join(RAIZ, ".github/workflows/ci.yml"), encoding="utf-8").read().splitlines()
    out, dentro = [], False
    for l in lineas:
        if re.match(r"^  [A-Za-z0-9_-]+:\s*$", l) or re.match(r"^\S", l):
            if dentro:
                break
            dentro = l.strip() == f"{job}:"
            continue
        if dentro and l.strip() and not l.strip().startswith("#"):
            out.append(l.rstrip())
    if not out:
        raise SystemExit(f"⛔ `ci.yml` no tiene el job `{job}`")
    return out


def ficheros_de(carpeta):
    out = []
    for raiz, subs, fs in os.walk(carpeta):
        subs[:] = sorted(s for s in subs if s not in SIN and not s.startswith("."))
        out += [os.path.join(raiz, f) for f in sorted(fs) if not f.endswith(".md")]
    return out


class Workspace(binarios.Workspace):
    """El de los binarios, con lo que una PRUEBA compila además: dev- y build-dependencies."""

    def __init__(self):
        super().__init__()
        for p, x in self.paquetes.items():
            t = tomllib.load(open(os.path.join(x["dir"], "Cargo.toml"), "rb"))
            secciones = [t.get(s, {}) for s in ("dev-dependencies", "build-dependencies")]
            for v in t.get("target", {}).values():
                secciones += [v.get(s, {}) for s in ("dev-dependencies", "build-dependencies")]
            for s in secciones:
                for clave, v in s.items():
                    if isinstance(v, dict) and "path" in v:
                        x["deps"].add(v.get("package", clave))

    def ficheros(self, paquete):
        d = self.paquetes[paquete]["dir"]
        propios, fuera = ficheros_de(d), set()
        for f in propios:
            if f.endswith(".rs"):
                r = rel(f).split("/")
                if len(r) > 2 and r[2] in ("tests", "benches", "examples"):
                    fuera |= self.incluidos_todo(f)   # aquí no hay `#[cfg(test)]` que quitar
                else:
                    fuera |= self.incluidos(f, d)
        return propios, sorted(fuera - set(propios))

    @staticmethod
    def incluidos_todo(fichero):
        out = set()
        for ruta in binarios.INCLUYE.findall(open(fichero, encoding="utf-8").read()):
            a = os.path.normpath(os.path.join(os.path.dirname(fichero), ruta))
            if not os.path.isfile(a):
                raise SystemExit(f"⛔ {rel(fichero)} incluye `{ruta}` y no existe")
            out.add(a)
        return out


def entradas(job, ws=None):
    ws = ws or Workspace()
    texto = bloque(job)
    out = [f"job {job}"] + [f"  {l}" for l in texto] + [f"{sha(os.path.join(RAIZ, c))}  {c}" for c in COMUNES]
    paquetes, rutas, leidos = set(), set(), set()
    pendientes = ["\n".join(texto)]
    while pendientes:
        t = pendientes.pop()
        for a, b in PAQUETE.findall(t):
            paquetes.add(a or b)
        for r in RUTA.findall(t):
            partes = r.split("/")
            if partes[0] == "pruebas-de-fuego" and len(partes) > 2:
                r = "/".join(partes[:2])
            a = os.path.join(RAIZ, r)
            if not os.path.exists(a):
                raise SystemExit(f"⛔ `{job}` nombra `{r}` y no existe")
            for f in (ficheros_de(a) if os.path.isdir(a) else [a]):
                rutas.add(f)
                if f.endswith(GUION) and f not in leidos:
                    leidos.add(f)
                    pendientes.append(open(f, encoding="utf-8", errors="replace").read())
    for p in sorted(paquetes):
        if p not in ws.paquetes:
            raise SystemExit(f"⛔ `{job}` nombra el paquete `{p}` y no está en crates/")
    out.append("paquetes " + " ".join(sorted(paquetes)))
    vistos = set()
    for p in sorted({q for p in paquetes for q in ws.cierre(p)}):
        propios, fuera = ws.ficheros(p)
        out.append(f"[{p}]")
        out += [f"  {sha(f)}  {rel(f)}" for f in propios]
        out += [f"  {sha(f)}  {rel(f)}  (incluido)" for f in fuera]
        vistos |= set(propios)
    out.append("[rutas]")
    out += [f"  {sha(f)}  {rel(f)}" for f in sorted(rutas - vistos)]
    return out


def clave(job, ws=None):
    return f"prueba-{job}-" + hashlib.sha256("\n".join(entradas(job, ws)).encode()).hexdigest()[:16]


def main():
    args = sys.argv[1:]
    if len(args) == 2 and args[0] == "--explicar":
        e = entradas(args[1])
        print("\n".join(e))
        print(f"\nclave {clave(args[1])}")
    elif len(args) >= 2 and args[0] == "--github":
        ws = Workspace()
        for j in args[1:]:
            print(f"{j}={clave(j, ws)}")
    elif args and not any(a.startswith("--") for a in args):
        ws = Workspace()
        for j in args:
            print(f"{j} {clave(j, ws)}")
    else:
        raise SystemExit(__doc__)


if __name__ == "__main__":
    main()
