#!/usr/bin/env python3
"""LA HUELLA DE CADA IMAGEN (ADR 0060 B1·1, G4): 16 hex de todo lo que decide lo que una
etapa del Dockerfile produce. Si no cambia, la imagen es la misma: el CI le pone la etiqueta del
commit a `<imagen>:h-<huella>`, que ya está en el registro, en vez de construirla y subirla.

    python3 ci/huella-de-la-imagen.py --binarios <h> puesto-python      # una
    python3 ci/huella-de-la-imagen.py --binarios <h> --todas            # las del CI, «imagen huella»
    python3 ci/huella-de-la-imagen.py --binarios <h> --explicar serve   # de qué sale
    python3 ci/huella-de-la-imagen.py --comprobar                       # que cuadra con el Dockerfile

Lo que entra, siguiendo la cadena de la etapa (su `FROM` y cada `COPY --from`):
  · el texto de cada etapa de la cadena, sin comentarios ni líneas en blanco (un comentario
    nuevo no rehace nada), y los `ARG` de antes del primer `FROM`;
  · el contenido de lo que copia del contexto (`COPY`/`ADD` sin `--from`), fichero a fichero,
    con su ruta: lo que el contexto deja pasar (`.dockerignore`);
  · el nombre de cada imagen de fuera (`FROM python:3.14-slim`, `COPY --from=node:24-slim`);
  · y, en vez de la etapa `bin` —`FROM ${BINARIOS}`—, la huella de los binarios
    (`ci/huella-de-los-binarios.sh`), que el CI pasa con `--binarios`.

⚠️ Una imagen de fuera se nombra por su etiqueta: si `python:3.14-slim` cambia por dentro, la
huella no. Para rehacerlas (un parche de seguridad de la base), `FORZAR=<lo que sea>` entra en
todas las huellas.
"""
import fnmatch
import hashlib
import os
import re
import sys

# `ORE_RAIZ`: otro árbol (la prueba, `ci/prueba-huella-de-la-imagen.py`, lo usa con una copia).
RAIZ = os.environ.get("ORE_RAIZ") or os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))
DOCKERFILE = os.path.join(RAIZ, "Dockerfile")

# Las que construye el CI: la etapa del Dockerfile y el nombre de la imagen en el registro. La
# misma lista que cloudbuild.yaml; `--comprobar` mira que cada etapa exista.
IMAGENES = {
    "ore": "ore", "informador": "ore-informador", "capa-jvm": "capa-jvm", "capa-node": "capa-node",
    "capa-python": "capa-python", "idp": "idp", "drivers": "ore-drivers", "serve": "ore-serve",
    "iam": "ore-iam", "cofre": "ore-cofre", "postgres-plano": "ore-postgres",
    "puesto-python": "puesto-python", "puesto-node": "puesto-node", "puesto-jvm": "puesto-jvm",
}
BIN = "bin"  # `FROM ${BINARIOS} AS bin`: la que sustituye la huella de los binarios


def logicas(texto):
    """Las instrucciones del Dockerfile: sin comentarios, con las continuaciones unidas."""
    out, acc = [], ""
    for linea in texto.splitlines():
        s = linea.strip()
        if not acc and (not s or s.startswith("#")):
            continue
        if acc and s.startswith("#"):
            continue  # un comentario dentro de un RUN partido
        if s.endswith("\\"):
            acc += s[:-1].rstrip() + " "
            continue
        out.append((acc + s).strip())
        acc = ""
    if acc:
        out.append(acc.strip())
    return out


def etapas(instrucciones):
    """{nombre: {"from": base, "lineas": [...]}} y los ARG globales."""
    globales, todas, actual = [], {}, None
    for i, ins in enumerate(instrucciones):
        m = re.match(r"(?i)FROM\s+(?:--platform=\S+\s+)?(\S+)(?:\s+AS\s+(\S+))?$", ins)
        if m:
            nombre = m.group(2) or f"#{i}"
            actual = todas[nombre] = {"from": m.group(1), "lineas": [ins]}
            continue
        if actual is None:
            globales.append(ins)
        else:
            actual["lineas"].append(ins)
    return globales, todas


def ignorados():
    """Los patrones de `.dockerignore`, con sus excepciones (`!`), en orden."""
    p = os.path.join(RAIZ, ".dockerignore")
    if not os.path.exists(p):
        return []
    reglas = []
    for l in open(p, encoding="utf-8"):
        l = l.strip()
        if l and not l.startswith("#"):
            neg = l.startswith("!")
            reglas.append((neg, l.lstrip("!").rstrip("/")))
    return reglas


def entra(ruta, reglas):
    """Como Docker: el último patrón que casa decide; un patrón casa la ruta o un padre suyo."""
    dentro = True
    partes = ruta.split("/")
    for neg, pat in reglas:
        if any(fnmatch.fnmatchcase("/".join(partes[: k + 1]), pat) for k in range(len(partes))):
            dentro = neg
    return dentro


def ficheros(fuente, reglas):
    base = os.path.join(RAIZ, fuente)
    if os.path.isfile(base):
        rel = os.path.relpath(base, RAIZ).replace(os.sep, "/")
        return [rel] if entra(rel, reglas) else []
    if not os.path.isdir(base):
        raise SystemExit(f"⛔ el Dockerfile copia `{fuente}` y no existe")
    out = []
    for d, subs, fs in os.walk(base):
        subs[:] = sorted(s for s in subs if s != ".git")
        for f in fs:
            rel = os.path.relpath(os.path.join(d, f), RAIZ).replace(os.sep, "/")
            if entra(rel, reglas):
                out.append(rel)
    return sorted(out)


def fuentes_de_copy(ins):
    """(--from o None, [fuentes]) de un COPY/ADD."""
    trozos = ins.split()
    desde, resto = None, []
    for t in trozos[1:]:
        if t.startswith("--from="):
            desde = t[len("--from="):]
        elif t.startswith("--"):
            continue
        else:
            resto.append(t)
    return desde, resto[:-1]


class Huella:
    def __init__(self, binarios):
        texto = open(DOCKERFILE, encoding="utf-8").read()
        self.globales, self.etapas = etapas(logicas(texto))
        self.binarios = binarios
        self.reglas = ignorados()
        self.memo = {}

    def entradas(self, etapa, visto=None):
        """Lo que decide la etapa, como lista de líneas legibles (para hashear y para explicar)."""
        visto = visto or set()
        if etapa == BIN:
            if not self.binarios:
                raise SystemExit("⛔ esta imagen copia binarios: falta --binarios <huella>")
            return [f"binarios h-{self.binarios}"]
        if etapa not in self.etapas:
            return [f"imagen de fuera {etapa}"]
        if etapa in visto:
            raise SystemExit(f"⛔ ciclo en el Dockerfile por {etapa}")
        visto = visto | {etapa}
        e = self.etapas[etapa]
        out = []
        base = e["from"]
        if base != "scratch":
            out += [f"[{etapa}] FROM:"] + ["  " + x for x in self.entradas(base if base in self.etapas else
                                                                        (BIN if "${BINARIOS}" in base else base), visto)]
        for ins in e["lineas"]:
            out.append(f"[{etapa}] {ins}")
            if re.match(r"(?i)(COPY|ADD)\s", ins):
                desde, fuentes = fuentes_de_copy(ins)
                if desde:
                    out += ["  " + x for x in self.entradas(desde, visto)]
                else:
                    for f in fuentes:
                        for rel in ficheros(f, self.reglas):
                            h = hashlib.sha256(open(os.path.join(RAIZ, rel), "rb").read()).hexdigest()
                            out.append(f"  {h}  {rel}")
        return out

    def de(self, etapa):
        if etapa not in self.memo:
            todo = self.globales + self.entradas(etapa) + [f"FORZAR={os.environ.get('FORZAR', '')}"]
            self.memo[etapa] = hashlib.sha256("\n".join(todo).encode()).hexdigest()[:16]
        return self.memo[etapa]


def main():
    args = sys.argv[1:]
    binarios = None
    if "--binarios" in args:
        i = args.index("--binarios")
        binarios = args[i + 1]
        del args[i:i + 2]
    if args == ["--comprobar"]:
        h = Huella("0" * 16)
        mal = [e for e in IMAGENES if e not in h.etapas]
        for e in IMAGENES:
            if e in h.etapas:
                h.de(e)  # que cada COPY exista y la cadena se pueda seguir
        if mal:
            raise SystemExit(f"⛔ etapas que el CI construye y el Dockerfile no tiene: {mal}")
        print(f"huellas: las {len(IMAGENES)} imágenes del CI cuadran con el Dockerfile")
        return
    h = Huella(binarios)
    if args == ["--todas"]:
        for etapa, imagen in IMAGENES.items():
            print(f"{etapa} {imagen} {h.de(etapa)}")
    elif len(args) == 2 and args[0] == "--explicar":
        print("\n".join(h.globales + h.entradas(args[1])))
        print(f"\nhuella {h.de(args[1])}")
    elif len(args) == 1:
        print(h.de(args[0]))
    else:
        raise SystemExit(__doc__)


if __name__ == "__main__":
    main()
