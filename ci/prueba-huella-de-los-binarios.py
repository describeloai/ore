#!/usr/bin/env python3
"""La prueba de `ci/huella-de-los-binarios.py` (ADR 0060 B2): sobre una copia de lo que cargo
lee, cada cambio mueve la huella de los binarios que lo llevan, y SÓLO de esos."""
import os
import shutil
import subprocess
import sys
import tempfile

AQUI = os.path.dirname(os.path.abspath(__file__))
RAIZ = os.path.normpath(os.path.join(AQUI, ".."))
COPIAR = ["Cargo.toml", "Cargo.lock", "rust-toolchain.toml", "Dockerfile", "ci/compilar-binarios.sh", "crates", "puesto"]
TODOS = None  # los 24, de la primera corrida
# Los que no dependen de ore-core: desde CI C4·3 (8bedd08d) el vocabulario vive en ore-base, y sólo
# ore-cli, ore-serve y lo que enlaza ore-registry siguen usando ore-core.
SIN_CORE = {"ore-firmar-s3", "ore-packages", "ore-redimensionar",
            "ore-asumir-rol", "ore-cofre", "ore-copiador", "ore-federation", "ore-fetch", "ore-iam", "ore-invoke",
            "ore-log", "ore-medios", "ore-postgres", "ore-read-azure", "ore-read-bigquery", "ore-read-cassandra",
            "ore-read-gcs", "ore-read-hbase", "ore-read-jsonl", "ore-read-mongodb", "ore-read-phoenix",
            "ore-read-postgres", "ore-read-s3", "ore-read-sftp", "ore-read-sharepoint", "ore-sign", "ore-store-gcs",
            "ore-store-r2", "ore-teselas"}


def huellas(raiz):
    env = dict(os.environ, ORE_RAIZ=raiz, PYTHONIOENCODING="utf-8")
    out = subprocess.run([sys.executable, os.path.join(AQUI, "huella-de-los-binarios.py"), "--todas"],
                         env=env, capture_output=True, text=True, check=True).stdout
    return {l.split()[0]: l.split()[2] for l in out.splitlines()}


# Lo que cada caso toca, para dejarlo como estaba: copiar el árbol por caso eran 4 min.
DESHACER = []


def guardar(p):
    DESHACER.append((p, open(p, "rb").read() if os.path.exists(p) else None))


def deshacer():
    while DESHACER:
        p, antes = DESHACER.pop()
        if antes is None:
            os.remove(p)
        else:
            open(p, "wb").write(antes)


def tocar(raiz, ruta, texto="\n// tocado por la prueba\n"):
    p = os.path.join(raiz, ruta)
    os.makedirs(os.path.dirname(p), exist_ok=True)
    guardar(p)
    with open(p, "a", encoding="utf-8") as f:
        f.write(texto)


def cambiar(raiz, ruta, viejo, nuevo):
    p = os.path.join(raiz, ruta)
    guardar(p)
    t = open(p, encoding="utf-8").read()
    assert t.count(viejo) >= 1, viejo
    open(p, "w", encoding="utf-8").write(t.replace(viejo, nuevo, 1))


def todos_menos(*fuera):
    return lambda todos: set(todos) - set(fuera)


CASOS = [
    ("el código de ore-serve", lambda r: tocar(r, "crates/ore-serve/src/main.rs"), {"ore-serve"}),
    ("el de ore-store (dos binarios, y ore-medios y ore-teselas, que lo usan)", lambda r: tocar(r, "crates/ore-store/src/lib.rs"),
     {"ore-store-r2", "ore-store-gcs", "ore-medios", "ore-teselas"}),
    ("el de ore-core (todos menos los que no lo usan)", lambda r: tocar(r, "crates/ore-core/src/lib.rs"), todos_menos(*SIN_CORE)),
    ("el Cargo.toml de ore-iam (y ore-cofre, que depende de él)", lambda r: tocar(r, "crates/ore-iam/Cargo.toml", "\n# x\n"),
     {"ore-iam", "ore-cofre"}),
    ("una lista que ore-serve incluye", lambda r: tocar(r, "puesto/python/sugeridas.txt", "\n"), {"ore-serve"}),
    ("un fichero que sólo incluye una prueba", lambda r: tocar(r, "puesto/python/ore/__init__.py", "\n# x\n"), set()),
    ("un .md de un paquete", lambda r: tocar(r, "crates/ore-serve/NOTAS.md", "x\n"), set()),
    ("una prueba de integración (tests/)", lambda r: tocar(r, "crates/ore-store/tests/nueva.rs"), set()),
    ("un ejemplo (examples/)", lambda r: tocar(r, "crates/ore-store/examples/nuevo.rs"), set()),
    ("Cargo.lock", lambda r: tocar(r, "Cargo.lock", "\n"), todos_menos()),
    ("la toolchain", lambda r: tocar(r, "rust-toolchain.toml", "\n# x\n"), todos_menos()),
    ("el guion que compila", lambda r: tocar(r, "ci/compilar-binarios.sh", "\n# x\n"), todos_menos()),
    ("un comentario del Dockerfile", lambda r: tocar(r, "Dockerfile", "\n# solo un comentario\n"), set()),
    ("la etapa herramientas del Dockerfile",
     lambda r: cambiar(r, "Dockerfile", "RUN apk add --no-cache musl-dev", "RUN apk add --no-cache musl-dev file"), todos_menos()),
    ("una etapa que no compila (serve)",
     lambda r: cambiar(r, "Dockerfile", "RUN apk add --no-cache git ca-certificates", "RUN apk add --no-cache git ca-certificates curl"),
     set()),
]


def main():
    mal = 0
    with tempfile.TemporaryDirectory() as t:
        base = os.path.join(t, "base")
        for c in COPIAR:
            o, d = os.path.join(RAIZ, c), os.path.join(base, c)
            os.makedirs(os.path.dirname(d), exist_ok=True)
            if os.path.isdir(o):
                shutil.copytree(o, d, ignore=shutil.ignore_patterns("target", ".git"))
            else:
                shutil.copy2(o, d)
        antes = huellas(base)
        if huellas(base) != antes:
            print("✗ la huella no es estable: dos corridas sobre lo mismo dan otra")
            return 1
        for nombre, hacer, esperado in CASOS:
            if callable(esperado):
                esperado = esperado(antes)
            hacer(base)
            despues = huellas(base)
            deshacer()
            cambian = {b for b in antes if antes[b] != despues.get(b)}
            if cambian == esperado:
                print(f"✓ {nombre}: {len(cambian)} cambian")
            else:
                mal += 1
                print(f"✗ {nombre}: de más {sorted(cambian - esperado)}, de menos {sorted(esperado - cambian)}")
    print(f"\n{len(CASOS) - mal}/{len(CASOS)}")
    return 1 if mal else 0


if __name__ == "__main__":
    sys.exit(main())
