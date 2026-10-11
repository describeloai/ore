#!/usr/bin/env python3
"""LA HUELLA DE CADA BINARIO (ADR 0060 B2): 16 hex de todo lo que decide lo que `cargo` compila
para UN binario de las imágenes. Si no cambia, el binario es el mismo: el CI lo saca de su caché y
no lo compila, y las imágenes que lo copian no cambian por él (`ci/huella-de-la-imagen.py`).

    python3 ci/huella-de-los-binarios.py --todas              # «binario paquete huella», uno por línea
    python3 ci/huella-de-los-binarios.py --mapa               # {binario: huella}, en JSON
    python3 ci/huella-de-los-binarios.py --explicar ore-serve # de qué sale
    python3 ci/huella-de-los-binarios.py --comprobar          # que todo lo que lee se ve
    python3 ci/huella-de-los-binarios.py --faltan <mapa> <dir> "<binarios>"   # los que la caché no tiene
    python3 ci/huella-de-los-binarios.py --apuntar <mapa> <dir> "<binarios>"  # y que ya los tiene

Lo que entra en la de un binario:
  · lo común a todos: `Cargo.toml` y `Cargo.lock` de la raíz, `rust-toolchain.toml`, el guion que
    compila y las etapas `herramientas`…`build` del Dockerfile (el compilador y cómo se llama);
  · los ficheros de SU paquete y de cada paquete del workspace del que depende (`[dependencies]`
    con `path`, también los de `[target.*]`; las `[dev-dependencies]` no entran en un binario),
    sin `tests/`, `examples/`, `benches/` ni los `.md`;
  · lo que el código de esos paquetes incluye de FUERA de su carpeta (`include_str!`,
    `include_bytes!`), leído del código: fuera de un `#[cfg(test)] mod`, que no se compila
    para la imagen.

Los binarios son los que el Dockerfile copia (`COPY --from=bin /b/<binario>`): ésos y sólo ésos.

Antes (2026-10-03 → B2) era UNA huella para los veinticuatro: tocar `ore-serve` recompilaba el
lector de SFTP y rehacía las catorce imágenes. Por el CONTENIDO de los ficheros y no por git, como
la de las imágenes: la calcula igual el job de GitHub que una construcción a mano.
"""
import hashlib
import json
import os
import re
import sys
import tomllib

# `ORE_RAIZ`: otro árbol (la prueba, `ci/prueba-huella-de-los-binarios.py`, lo usa con una copia).
RAIZ = os.environ.get("ORE_RAIZ") or os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))
COMUNES = ["Cargo.toml", "Cargo.lock", "rust-toolchain.toml", "ci/compilar-binarios.sh"]
FUERA_DEL_PAQUETE = {"tests", "examples", "benches", "target"}
INCLUYE = re.compile(r'include_(?:str|bytes)!\(\s*"([^"]+)"\s*\)')
PRUEBAS = re.compile(r"#\[cfg\(test\)\]\s*(?:#\[[^\]]*\]\s*)*(?:pub(?:\([^)]*\))?\s+)?mod\s+\w+\s*\{")


def sha(ruta):
    return hashlib.sha256(open(ruta, "rb").read()).hexdigest()


def rel(ruta):
    return os.path.relpath(ruta, RAIZ).replace(os.sep, "/")


def herramientas():
    """Las etapas del Dockerfile que compilan, de `herramientas` a la de los binarios, sin comentarios."""
    texto = open(os.path.join(RAIZ, "Dockerfile"), encoding="utf-8").read()
    dentro, out = False, []
    for l in texto.splitlines():
        if re.match(r"^FROM rust:.* AS herramientas$", l):
            dentro = True
        elif re.match(r"^FROM scratch AS binarios$", l):
            dentro = False
        s = l.strip()
        if dentro and s and not s.startswith("#"):
            out.append(s)
    if not out:
        raise SystemExit("⛔ el Dockerfile no tiene la etapa `herramientas`")
    return out


def copiados():
    """Los binarios que el Dockerfile copia de `bin`, en orden y sin repetir."""
    texto = open(os.path.join(RAIZ, "Dockerfile"), encoding="utf-8").read()
    vistos = []
    for b in re.findall(r"COPY\s+--from=bin\s+/b/(\S+)", texto):
        if b not in vistos:
            vistos.append(b)
    return vistos


class Workspace:
    def __init__(self):
        self.paquetes = {}  # nombre → {"dir", "deps", "bins"}
        base = os.path.join(RAIZ, "crates")
        for d in sorted(os.listdir(base)):
            m = os.path.join(base, d, "Cargo.toml")
            if not os.path.isfile(m):
                continue
            t = tomllib.load(open(m, "rb"))
            nombre = t["package"]["name"]
            secciones = [t.get("dependencies", {})] + [v.get("dependencies", {}) for v in t.get("target", {}).values()]
            deps = set()
            for s in secciones:
                for clave, v in s.items():
                    if isinstance(v, dict) and "path" in v:
                        deps.add(v.get("package", clave))
            bins = [b["name"] for b in t.get("bin", [])]
            if not bins and os.path.isfile(os.path.join(base, d, "src/main.rs")):
                bins = [nombre]
            self.paquetes[nombre] = {"dir": os.path.join(base, d), "deps": deps, "bins": bins}
        self.de_binario = {b: p for p, x in self.paquetes.items() for b in x["bins"]}

    def cierre(self, paquete):
        """El paquete y todos los del workspace de los que depende, en orden."""
        visto, pila = set(), [paquete]
        while pila:
            p = pila.pop()
            if p in visto:
                continue
            if p not in self.paquetes:
                raise SystemExit(f"⛔ `{p}` es una dependencia con `path` que no está en crates/")
            visto.add(p)
            pila += sorted(self.paquetes[p]["deps"])
        return sorted(visto)

    def ficheros(self, paquete):
        """Los ficheros de un paquete que entran en su binario, y lo que su código incluye de fuera."""
        d = self.paquetes[paquete]["dir"]
        propios, fuera = [], set()
        for raiz, subs, fs in os.walk(d):
            if raiz == d:
                subs[:] = [s for s in subs if s not in FUERA_DEL_PAQUETE]
            subs[:] = sorted(s for s in subs if not s.startswith("."))
            for f in sorted(fs):
                if f.endswith(".md"):
                    continue
                p = os.path.join(raiz, f)
                propios.append(p)
                if f.endswith(".rs"):
                    fuera |= self.incluidos(p, d)
        return propios, sorted(fuera)

    @staticmethod
    def incluidos(fichero, dir_paquete):
        """Lo que `fichero` incluye de fuera de su paquete, salvo dentro de un `#[cfg(test)] mod`."""
        texto = open(fichero, encoding="utf-8").read()
        m = PRUEBAS.search(texto)
        prod = texto[: m.start()] if m else texto
        out = set()
        for ruta in INCLUYE.findall(prod):
            a = os.path.normpath(os.path.join(os.path.dirname(fichero), ruta))
            if not os.path.isfile(a):
                raise SystemExit(f"⛔ {rel(fichero)} incluye `{ruta}` y no existe")
            r = os.path.relpath(a, dir_paquete).replace(os.sep, "/")
            # Fuera: de otra carpeta, o de una que `ficheros` no recorre (`tests/`…).
            if r.startswith("../") or r.split("/")[0] in FUERA_DEL_PAQUETE:
                out.add(a)
        return out


class Huellas:
    def __init__(self):
        self.ws = Workspace()
        self.comun = [f"{sha(os.path.join(RAIZ, c))}  {c}" for c in COMUNES] + herramientas()
        self.memo = {}
        self.de_paquete = {}  # cada paquete se lee una vez, aunque lo usen veinte binarios

    def entradas(self, binario):
        if binario not in self.ws.de_binario:
            raise SystemExit(f"⛔ ningún paquete de crates/ da el binario `{binario}`")
        paquete = self.ws.de_binario[binario]
        out = [f"binario {binario} del paquete {paquete}"] + self.comun
        for p in self.ws.cierre(paquete):
            if p not in self.de_paquete:
                propios, fuera = self.ws.ficheros(p)
                self.de_paquete[p] = [f"[{p}]"] + [f"  {sha(f)}  {rel(f)}" for f in propios] +                     [f"  {sha(f)}  {rel(f)}  (incluido)" for f in fuera]
            out += self.de_paquete[p]
        return out

    def de(self, binario):
        if binario not in self.memo:
            self.memo[binario] = hashlib.sha256("\n".join(self.entradas(binario)).encode()).hexdigest()[:16]
        return self.memo[binario]

    def todas(self):
        return {b: self.de(b) for b in copiados()}


def comprobar():
    h = Huellas()
    bins = copiados()
    mal = [b for b in bins if b not in h.ws.de_binario]
    if mal:
        raise SystemExit(f"⛔ el Dockerfile copia binarios que ningún paquete da: {mal}")
    # El guion compila exactamente los que el Dockerfile copia.
    guion = open(os.path.join(RAIZ, "ci/compilar-binarios.sh"), encoding="utf-8").read()
    m = re.search(r'^TODOS="([^"]*)"', guion, re.M)
    if not m:
        raise SystemExit("⛔ ci/compilar-binarios.sh no dice TODOS=\"…\"")
    lista = m.group(1).split()
    if sorted(lista) != sorted(bins):
        raise SystemExit(f"⛔ TODOS de compilar-binarios.sh y el Dockerfile no cuadran: "
                         f"sólo en el guion {sorted(set(lista) - set(bins))}, sólo en el Dockerfile {sorted(set(bins) - set(lista))}")
    for b in bins:
        h.de(b)
    print(f"huellas: los {len(bins)} binarios que el Dockerfile copia salen de {len({h.ws.de_binario[b] for b in bins})} paquetes, y se ve todo lo que leen")


def manifiesto(dir_):
    """Las huellas de lo que trae la caché de los binarios (`<dir>/.huellas.json`)."""
    p = os.path.join(dir_, ".huellas.json")
    return json.load(open(p, encoding="utf-8")) if os.path.isfile(p) else {}


def main():
    args = sys.argv[1:]
    if args == ["--comprobar"]:
        comprobar()
    elif args == ["--todas"]:
        h = Huellas()
        for b, x in h.todas().items():
            print(f"{b} {h.ws.de_binario[b]} {x}")
    elif args == ["--mapa"]:
        print(json.dumps(Huellas().todas(), sort_keys=True))
    elif len(args) == 4 and args[0] == "--faltan":
        # De `necesarios`, los que la caché (`dir`, con su `.huellas.json`) no tiene o no casan.
        _, mapa, dir_, necesarios = args
        quiero = json.load(open(mapa, encoding="utf-8"))
        tengo = manifiesto(dir_)
        print(" ".join(b for b in necesarios.split()
                       if tengo.get(b) != quiero[b] or not os.path.isfile(os.path.join(dir_, b))))
    elif len(args) == 4 and args[0] == "--apuntar":
        # Que `compilados` ya están en la caché, con la huella de ahora.
        _, mapa, dir_, compilados = args
        quiero = json.load(open(mapa, encoding="utf-8"))
        tengo = manifiesto(dir_)
        tengo.update({b: quiero[b] for b in compilados.split()})
        json.dump(tengo, open(os.path.join(dir_, ".huellas.json"), "w", encoding="utf-8"), sort_keys=True, indent=1)
        print(f"  en la caché: {len(tengo)} binarios")
    elif len(args) == 2 and args[0] == "--explicar":
        h = Huellas()
        print("\n".join(h.entradas(args[1])))
        print(f"\nhuella {h.de(args[1])}")
    elif len(args) == 1 and not args[0].startswith("--"):
        print(Huellas().de(args[0]))
    else:
        raise SystemExit(__doc__)


if __name__ == "__main__":
    main()
