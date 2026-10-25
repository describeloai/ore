#!/usr/bin/env python3
"""La prueba de `ci/huella-de-las-pruebas.py`: sobre una copia de lo que las pruebas largas leen,
cada cambio mueve la huella de las que lo prueban, y SÓLO de esas. Lo que importa en las dos
direcciones: tocar `ore-serve` no hace esperar a los conectores, y tocar algo que un conector usa
no se cuela sin su prueba."""
import os
import shutil
import subprocess
import sys
import tempfile

AQUI = os.path.dirname(os.path.abspath(__file__))
RAIZ = os.path.normpath(os.path.join(AQUI, ".."))
COPIAR = ["Cargo.toml", "Cargo.lock", "rust-toolchain.toml", ".github/workflows/ci.yml", "ci", "crates",
          "pruebas-de-fuego", "puesto"]
JOBS = ["conectores", "hbase-kerberos"]
AMBAS, CON, NINGUNA = set(JOBS), {"conectores"}, set()


def huellas(raiz):
    env = dict(os.environ, ORE_RAIZ=raiz, PYTHONIOENCODING="utf-8")
    out = subprocess.run([sys.executable, os.path.join(AQUI, "huella-de-las-pruebas.py")] + JOBS,
                         env=env, capture_output=True, text=True, check=True).stdout
    return dict(l.split() for l in out.splitlines())


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
    guardar(p)
    with open(p, "a", encoding="utf-8") as f:
        f.write(texto)


def cambiar(raiz, ruta, viejo, nuevo):
    p = os.path.join(raiz, ruta)
    guardar(p)
    t = open(p, encoding="utf-8").read()
    assert t.count(viejo) == 1, viejo
    open(p, "w", encoding="utf-8", newline="\n").write(t.replace(viejo, nuevo))


CI = ".github/workflows/ci.yml"
CASOS = [
    # Lo del 2026-10-23: `ore-serve`, `puesto/python` y `malla` esperaron 40 min por los conectores.
    ("ore-serve", lambda r: tocar(r, "crates/ore-serve/src/main.rs"), NINGUNA),
    ("puesto/python", lambda r: tocar(r, "puesto/python/provisto.txt", "\n"), NINGUNA),
    ("ore-cli", lambda r: tocar(r, "crates/ore-cli/src/main.rs"), NINGUNA),
    ("ore-store", lambda r: tocar(r, "crates/ore-store/src/almacen.rs"), NINGUNA),
    ("ore-core", lambda r: tocar(r, "crates/ore-core/src/lib.rs"), NINGUNA),
    ("un .md de ore-hbase", lambda r: tocar(r, "crates/ore-hbase/NOTAS.md", "x\n"), NINGUNA),
    ("el job construir", lambda r: cambiar(r, CI, "xargs -P 5 -L 1", "xargs -P 6 -L 1"), NINGUNA),
    ("un comentario del job conectores",
     lambda r: cambiar(r, CI, "      # ADR 0069 M3: MongoDB. Los tipos", "      # tocado\n      # ADR 0069 M3: MongoDB. Los tipos"),
     NINGUNA),
    # Lo que tiene que seguir esperando.
    ("ore-hbase", lambda r: tocar(r, "crates/ore-hbase/src/lib.rs"), AMBAS),
    ("una prueba de ore-hbase", lambda r: tocar(r, "crates/ore-hbase/tests/laboratorio.rs"), AMBAS),
    ("ore-hbase-rpc", lambda r: tocar(r, "crates/ore-hbase-rpc/src/lib.rs"), AMBAS),
    # Desde CI C4·3 (8bedd08d) los conectores usan ore-base, no ore-core: lo que mueve a ambos es ore-base.
    ("ore-base", lambda r: tocar(r, "crates/ore-base/src/lib.rs"), AMBAS),
    ("ore-driver", lambda r: tocar(r, "crates/ore-driver/src/lib.rs"), AMBAS),
    ("Cargo.lock", lambda r: tocar(r, "Cargo.lock", "\n"), AMBAS),
    ("el laboratorio de Kerberos", lambda r: tocar(r, "pruebas-de-fuego/hbase/kerberos/kdc.sh", "\n"), AMBAS),
    ("una traza de HBase que se incluye", lambda r: tocar(r, "pruebas-de-fuego/hbase/trazas/nativo.json", "\n"), AMBAS),
    ("ore-cassandra", lambda r: tocar(r, "crates/ore-cassandra/src/lib.rs"), CON),
    ("ore-avatica", lambda r: tocar(r, "crates/ore-avatica/src/lib.rs"), CON),
    ("ore-conector-kit", lambda r: tocar(r, "crates/ore-conector-kit/src/main.rs"), CON),
    ("ore-federation", lambda r: tocar(r, "crates/ore-federation/src/main.rs"), CON),
    ("ore-read-s3", lambda r: tocar(r, "crates/ore-read-s3/src/main.rs"), CON),
    ("ore-s3, que usa ore-read-s3", lambda r: tocar(r, "crates/ore-s3/src/lib.rs"), CON),
    ("la cinta de BigQuery", lambda r: tocar(r, "crates/ore-conector-kit/cintas/bigquery/LEEME", "x\n"), CON),
    ("el laboratorio de Phoenix", lambda r: tocar(r, "pruebas-de-fuego/phoenix/Dockerfile", "\n"), CON),
    ("el Graph de mentira", lambda r: tocar(r, "pruebas-de-fuego/graph-de-mentira.py", "\n"), CON),
    ("un paso del job conectores", lambda r: cambiar(r, CI, "--casos 1,2,3,4,5,6,8,9,10,11,12,13,14", "--casos 1,2,3,4,5,6,8,9,10,11,12,13"), CON),
    ("el paso del job hbase-kerberos", lambda r: cambiar(r, CI, "integrity authentication privacy", "integrity privacy"), {"hbase-kerberos"}),
]


def main():
    with tempfile.TemporaryDirectory() as tmp:
        for c in COPIAR:
            o, d = os.path.join(RAIZ, c), os.path.join(tmp, c)
            if os.path.isdir(o):
                shutil.copytree(o, d, ignore=shutil.ignore_patterns("target", "node_modules", ".venv", "__pycache__"))
            else:
                os.makedirs(os.path.dirname(d), exist_ok=True)
                shutil.copy2(o, d)
        base = huellas(tmp)
        if huellas(tmp) != base:
            raise SystemExit("⛔ la huella no es estable: dos corridas sobre lo mismo dan distinto")
        mal = 0
        for nombre, hacer, esperado in CASOS:
            hacer(tmp)
            ahora = huellas(tmp)
            deshacer()
            movidas = {j for j in JOBS if ahora[j] != base[j]}
            ok = movidas == esperado
            mal += not ok
            print(f"{'  ok' if ok else '⛔  '} {nombre}: mueve {sorted(movidas) or 'ninguna'}"
                  + ("" if ok else f", y tendría que mover {sorted(esperado) or 'ninguna'}"))
        if huellas(tmp) != base:
            raise SystemExit("⛔ deshacer no dejó la copia como estaba")
        if mal:
            raise SystemExit(f"⛔ {mal} de {len(CASOS)} casos")
        print(f"huellas de las pruebas: los {len(CASOS)} casos")


if __name__ == "__main__":
    main()
