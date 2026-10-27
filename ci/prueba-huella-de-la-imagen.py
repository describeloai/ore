#!/usr/bin/env python3
"""La prueba de `ci/huella-de-la-imagen.py` (ADR 0060 B1·1): sobre una copia de lo que el
Dockerfile copia, cada cambio mueve la huella de las imágenes que lo llevan, y SÓLO de esas."""
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile

AQUI = os.path.dirname(os.path.abspath(__file__))
RAIZ = os.path.normpath(os.path.join(AQUI, ".."))
COPIAR = ["Dockerfile", ".dockerignore", "puesto", "identidad", "malla/informar.sh", "malla/copiar-worker.sh", "ci/compilar-binarios.sh", "ci/marimo-frontend.ref"]
BIN = ["ore", "drivers", "serve", "iam", "cofre", "postgres-plano", "piramide", "puesto-python", "puesto-node",
       "puesto-jvm", "puesto-spark", "puesto-python-notebooks"]


# Los binarios que el Dockerfile copia: el mapa de las pruebas por binario.
BINARIOS = sorted(set(re.findall(r"COPY\s+--from=bin\s+/b/(\S+)", open(os.path.join(RAIZ, "Dockerfile"), encoding="utf-8").read())))


def huellas(raiz, binarios="0" * 16, forzar=""):
    env = dict(os.environ, ORE_RAIZ=raiz, FORZAR=forzar, PYTHONIOENCODING="utf-8")
    out = subprocess.run([sys.executable, os.path.join(AQUI, "huella-de-la-imagen.py"), "--binarios", binarios, "--todas"],
                         env=env, capture_output=True, text=True, check=True).stdout
    return {l.split()[0]: l.split()[2] for l in out.splitlines()}


def tocar(raiz, ruta, texto="\n# tocado por la prueba\n"):
    with open(os.path.join(raiz, ruta), "a", encoding="utf-8") as f:
        f.write(texto)


CASOS = [
    # ⭐ 0068 C2·2 / 0064: `puesto-spark` y `puesto-python-notebooks` salen de `puesto-python`,
    # así que lo que rehace una rehace las tres…
    ("un fichero del puesto de Python", lambda r: tocar(r, "puesto/python/agente.py"), {"puesto-python", "puesto-spark", "puesto-python-notebooks"}),
    # …y no al revés.
    # ⭐ 0064: el commit de `marimo-frontend` que fija el CI cambia la imagen de notebooks, y sólo ella.
    ("el commit de marimo-frontend", lambda r: tocar(r, "ci/marimo-frontend.ref", "\n"), {"puesto-python-notebooks"}),
    ("la lista de jars del puesto pesado", lambda r: tocar(r, "puesto/spark/jars.txt"), {"puesto-spark"}),
    ("la lista de Node (la capa y el puesto)", lambda r: tocar(r, "puesto/node/provisto.txt", "\n"), {"capa-node", "puesto-node"}),
    ("el guion del informador", lambda r: tocar(r, "malla/informar.sh"), {"informador"}),
    ("el guion del copiador", lambda r: tocar(r, "malla/copiar-worker.sh"), {"drivers"}),
    ("el tema del login", lambda r: tocar(r, next(os.path.join(d, f)[len(r) + 1:] for d, _, fs in os.walk(os.path.join(r, "identidad/tema")) for f in fs)), {"idp"}),
    ("un comentario del Dockerfile", lambda r: tocar(r, "Dockerfile", "\n# solo un comentario\n"), set()),
    ("un fichero que el contexto no deja pasar", lambda r: os.makedirs(os.path.join(r, "docs"), exist_ok=True) or tocar(r, "docs/x.md"), set()),
    ("una etapa nueva que nadie usa", lambda r: tocar(r, "Dockerfile", "\nFROM alpine:3.22 AS otra\n"), set()),
    ("una instrucción de la etapa serve",
     lambda r: cambiar(r, "RUN apk add --no-cache git ca-certificates", "RUN apk add --no-cache git ca-certificates curl"), {"serve"}),
    ("una etapa de la que copia otra (pyright)", lambda r: cambiar(r, "ARG PYRIGHT=", "ARG PYRIGHT=9"), {"puesto-python", "puesto-spark", "puesto-python-notebooks"}),
    # ⭐ 0072 F2·3·2: la versión de tippecanoe rehace la imagen de la pirámide, y sólo ella.
    ("la versión de tippecanoe", lambda r: cambiar(r, "ARG TIPPECANOE=2.79.0", "ARG TIPPECANOE=2.80.0"), {"piramide"}),
]


def cambiar(raiz, viejo, nuevo):
    p = os.path.join(raiz, "Dockerfile")
    t = open(p, encoding="utf-8").read()
    assert t.count(viejo) == 1, viejo
    open(p, "w", encoding="utf-8").write(t.replace(viejo, nuevo))


def main():
    mal = 0
    with tempfile.TemporaryDirectory() as t:
        base = os.path.join(t, "base")
        for c in COPIAR:
            o, d = os.path.join(RAIZ, c), os.path.join(base, c)
            os.makedirs(os.path.dirname(d), exist_ok=True)
            (shutil.copytree if os.path.isdir(o) else shutil.copy2)(o, d)
        antes = huellas(base)
        casos = CASOS + [
            ("la huella de todos los binarios", None, set(BIN)),
            # ADR 0060 B2: una huella por binario; cada imagen, sólo las de lo que copia.
            ("la huella de ore-serve", None, {"serve"}),
            ("la huella de ore-store-gcs", None, {"serve", "drivers", "puesto-python", "puesto-node", "puesto-jvm", "puesto-spark", "puesto-python-notebooks"}),
            ("la huella de ore-iam", None, {"iam"}),
            ("la huella de ore-piramide", None, {"piramide"}),
            ("FORZAR", None, set(antes)),
        ]
        for i, (que, hacer, espera) in enumerate(casos):
            raiz = os.path.join(t, f"c{i}")
            shutil.copytree(base, raiz)
            if que == "la huella de todos los binarios":
                despues = huellas(raiz, binarios="1" * 16)
            elif que.startswith("la huella de ore-"):
                b = que.split()[-1]
                despues = huellas(raiz, binarios=json.dumps({x: ("1" if x == b else "0") * 16 for x in BINARIOS}))
            elif que == "FORZAR":
                despues = huellas(raiz, forzar="2026-10")
            else:
                hacer(raiz)
                despues = huellas(raiz)
            cambian = {k for k in antes if antes[k] != despues[k]}
            ok = cambian == espera
            mal |= not ok
            print(f"{'✓' if ok else '✗'} {que}: cambian {sorted(cambian) or 'ninguna'}" + ("" if ok else f" (se esperaba {sorted(espera)})"))
    sys.exit(mal)


if __name__ == "__main__":
    main()
