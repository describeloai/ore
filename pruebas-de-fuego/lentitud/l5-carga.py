"""L5 · ore-serve a escala: un árbol sintético grande y usuarios a la vez.

    python3 l5-carga.py sembrar --dir /lab --activos 1000 --paquetes 20 --ramas 50 --historia 200
    python3 l5-carga.py cargar  --base http://ore-serve:8080 --usuarios 10 --segundos 60 --ramas 50

`sembrar` deja una forja pelada en `<dir>/arbol.git` con:
  · `--activos` datasets escritos (documento en `packages/<p>/datasets/` y su puntero en
    `datasets/<p>/default/`), repartidos en `--paquetes`;
  · `--historia` commits en `main`, cada uno moviendo 10 punteros (como escriben los puestos);
  · `--ramas` ramas sacadas de puntos distintos de esa historia, cada una con 3 punteros
    suyos y un dataset nuevo;
  · y `main` avanzando 20 commits más después: las ramas quedan atrás y leerlas exige
    montarlas sobre `main` (la superposición de 0044 C.2 ③).
`cargar` lanza `--usuarios` hilos durante `--segundos`; cada uno pide, en una rama al azar
(o en `main`, uno de cada cinco), lo que pide la consola: `/assets`, `/fuentes`, `/arbol` y
`/ramas`. Saca p50/p95/máx y errores por ruta, en JSON.
"""
import argparse
import json
import os
import random
import statistics
import subprocess
import threading
import time
import urllib.error
import urllib.request

ENTORNO = dict(os.environ, GIT_AUTHOR_NAME="l5", GIT_AUTHOR_EMAIL="l5@sujeto.invalid",
               GIT_COMMITTER_NAME="l5", GIT_COMMITTER_EMAIL="l5@sujeto.invalid")


def git(d, *a):
    subprocess.run(["git", *a], cwd=d, env=ENTORNO, check=True, capture_output=True)


def puntero(p, n, v):
    return json.dumps({"estado": "copiada", "tabla": f"{p}.{n}", "dataset": f"catalogo/{p}/default/{n}",
                       "metadata_location": f"gs://l5/catalogo/{p}/default/{n}/metadata/{v:05d}.metadata.json",
                       "snapshot": str(1000 + v), "filas": 10 * v, "uuid": f"00000000-0000-7000-8000-{v:012d}",
                       "operacion": f"op-{v}"}, indent=2) + "\n"


def documento(p, n):
    return (f"apiVersion: oos.dev/v1alpha12\nkind: Dataset\nmetadata: {{ name: {n}, namespace: {p} }}\n"
            f"spec:\n  owner: team:{p}\n  columns:\n    id: {{ type: Integer }}\n    pais: {{ type: String }}\n"
            f"  changes: {{ mode: append }}\n")


def escribe(raiz, rel, texto):
    f = os.path.join(raiz, rel)
    os.makedirs(os.path.dirname(f), exist_ok=True)
    with open(f, "w", encoding="utf-8", newline="\n") as fh:
        fh.write(texto)


def sembrar(a):
    t0 = time.time()
    forja, s = os.path.join(a.dir, "arbol.git"), os.path.join(a.dir, "semilla")
    subprocess.run(["rm", "-rf", forja, s], check=True)
    subprocess.run(["git", "init", "-q", "--bare", "-b", "main", forja], check=True)
    subprocess.run(["git", "clone", "-q", forja, s], check=True, capture_output=True)
    subprocess.run([a.ore, "init", ".", "--name", "l5"], cwd=s, check=True, capture_output=True)
    activos = []
    for i in range(a.activos):
        p = f"p{i % a.paquetes:03d}"
        n = f"d{i:05d}"
        activos.append((p, n))
        escribe(s, f"packages/{p}/datasets/{n}.yaml", documento(p, n))
        escribe(s, f"datasets/{p}/default/{n}.json", puntero(p, n, 0))
    for k in range(a.paquetes):
        p = f"p{k:03d}"
        escribe(s, f"packages/{p}/package.yaml",
                f"apiVersion: oos.dev/v1alpha1\nkind: Package\nmetadata: {{ name: {p}, version: 1.0.0, status: active, domain: l5 }}\n"
                f"spec: {{ owner: team:{p} }}\n")
    git(s, "add", "-A")
    git(s, "commit", "-qm", "semilla")
    tv = time.time()
    v = subprocess.run([a.ore, "validate", "."], cwd=s, capture_output=True, text=True)
    valida = {"codigo": v.returncode, "s": round(time.time() - tv, 1), "salida": (v.stdout + v.stderr)[-300:]}
    rnd = random.Random(5)
    for h in range(a.historia):
        for p, n in rnd.sample(activos, 10):
            escribe(s, f"datasets/{p}/default/{n}.json", puntero(p, n, h + 1))
        git(s, "commit", "-qam", f"historia {h}")
    git(s, "push", "-q", "origin", "HEAD:main")
    for b in range(a.ramas):
        git(s, "checkout", "-q", "-b", f"l5/r{b:03d}", f"main~{rnd.randrange(0, min(a.historia, 100))}")
        for p, n in rnd.sample(activos, 3):
            escribe(s, f"datasets/{p}/default/{n}.json", puntero(p, n, 50000 + b))
        p = activos[b % len(activos)][0]
        escribe(s, f"packages/{p}/datasets/rama{b:03d}.yaml", documento(p, f"rama{b:03d}"))
        escribe(s, f"datasets/{p}/default/rama{b:03d}.json", puntero(p, f"rama{b:03d}", 1))
        git(s, "add", "-A")
        git(s, "commit", "-qm", f"rama {b}")
        git(s, "push", "-q", "origin", f"l5/r{b:03d}")
        git(s, "checkout", "-q", "main")
    for h in range(20):
        for p, n in rnd.sample(activos, 10):
            escribe(s, f"datasets/{p}/default/{n}.json", puntero(p, n, a.historia + h + 1))
        git(s, "commit", "-qam", f"main avanza {h}")
    git(s, "push", "-q", "origin", "HEAD:main")
    print(json.dumps({"sembrado_s": round(time.time() - t0, 1), "activos": a.activos, "paquetes": a.paquetes,
                      "ramas": a.ramas, "commits_main": a.historia + 21, "validate": valida}))


# `/ramas` habla con la API de la forja (Gitea): una forja `file://` no la tiene.
RUTAS = ["/assets", "/fuentes", "/arbol"]


def cargar(a):
    ramas = [None] + [f"l5/r{b:03d}" for b in range(a.ramas)]
    fin = time.time() + a.segundos
    medidas, candado = [], threading.Lock()

    def usuario(i):
        rnd = random.Random(i)
        while time.time() < fin:
            ruta = rnd.choice(RUTAS)
            rama = None if rnd.random() < 0.2 else rnd.choice(ramas[1:])
            cab = {"x-ore-sujeto": "persona:ana"}
            if rama and ruta != "/ramas":
                cab["x-ore-rama"] = rama
            t = time.time()
            try:
                with urllib.request.urlopen(urllib.request.Request(a.base + ruta, headers=cab), timeout=300) as r:
                    r.read()
                    st = r.status
            except urllib.error.HTTPError as e:
                st = e.code
            except Exception:  # noqa: BLE001 — la conexión: el servidor caído o saturado
                st = 0
            with candado:
                medidas.append((ruta, rama is not None, st, (time.time() - t) * 1000))

    hs = [threading.Thread(target=usuario, args=(i,)) for i in range(a.usuarios)]
    t0 = time.time()
    for h in hs:
        h.start()
    for h in hs:
        h.join()
    dur = time.time() - t0
    out = {"usuarios": a.usuarios, "segundos": round(dur, 1), "peticiones": len(medidas),
           "por_segundo": round(len(medidas) / dur, 2), "rutas": {}}
    for ruta in RUTAS:
        for en_rama in (False, True):
            xs = [m for m in medidas if m[0] == ruta and m[1] == en_rama]
            if not xs:
                continue
            ms = sorted(m[3] for m in xs)
            err = sum(1 for m in xs if m[2] != 200)
            out["rutas"][ruta + (" (rama)" if en_rama else " (main)")] = {
                "n": len(xs), "errores": err, "p50": round(statistics.median(ms)),
                "p95": round(ms[max(0, int(len(ms) * 0.95) - 1)]), "max": round(ms[-1])}
    print(json.dumps(out))


if __name__ == "__main__":
    ap = argparse.ArgumentParser()
    sub = ap.add_subparsers(dest="verbo", required=True)
    s = sub.add_parser("sembrar")
    s.add_argument("--dir", default="/lab")
    s.add_argument("--activos", type=int, default=1000)
    s.add_argument("--paquetes", type=int, default=20)
    s.add_argument("--ramas", type=int, default=50)
    s.add_argument("--historia", type=int, default=200)
    s.add_argument("--ore", default="ore")
    c = sub.add_parser("cargar")
    c.add_argument("--base", required=True)
    c.add_argument("--usuarios", type=int, default=10)
    c.add_argument("--segundos", type=int, default=60)
    c.add_argument("--ramas", type=int, default=50)
    a = ap.parse_args()
    sembrar(a) if a.verbo == "sembrar" else cargar(a)
