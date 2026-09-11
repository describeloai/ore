# -*- coding: utf-8 -*-
"""`POST /paquetes`: una base es un paquete con alcance, inducido del catalogo de otro.

Medido el 2026-09-11. La consola tiene un modal —`CreateDatabaseModal.tsx`—
con un arbol de schemas y tablas y casillas. Su seleccion moria en `useState`:
no habia ruta a la que mandarla. Esta es la ruta, y esta medida la ejerce de
punta a punta contra un `ore-serve` de verdad, levantado aqui.

  A. `/esquema` da el nombre FISICO      lo que `--only` entiende
  B. `POST /paquetes` induce con alcance  y deja `discover.scope.json`
  C. `/paquetes` distingue base de fuente `scoped` y `source`
  D. lo que se niega, se niega BIEN       422/404/409 con motivo

Y no toca el origen: `discover --from` induce del catalogo que el Job dejo.
Por eso va por la misma via que `review` —clon, `ore`, commit— y no por la
cola de trabajo.
"""
import json
import os
import shutil
import socket
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request

for f in (sys.stdout, sys.stderr):
    try:
        f.reconfigure(errors="replace")
    except AttributeError:
        pass

RAIZ = os.environ.get("ORE_RAIZ", r"C:\ORE")


def binario(n):
    for c in ("release", "debug"):
        for ext in (".exe", ""):
            p = os.path.join(RAIZ, "target", c, n + ext)
            if os.path.exists(p):
                return p
    return shutil.which(n)


ORE = binario("ore")
SERVE = binario("ore-serve")
assert ORE and SERVE, "faltan binarios: cargo build -p ore-cli -p ore-serve"

fallos = []


def falla(que):
    fallos.append(que)
    print("  x  %s" % que)


def bien(que):
    print("  ok %s" % que)


def puerto_libre():
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    p = s.getsockname()[1]
    s.close()
    return p


def pide(metodo, ruta, cuerpo=None):
    datos = json.dumps(cuerpo).encode() if cuerpo is not None else None
    req = urllib.request.Request(BASE + ruta, data=datos, method=metodo)
    req.add_header("x-ore-sujeto", "persona:ana")
    if datos is not None:
        req.add_header("content-type", "application/json")
    try:
        with urllib.request.urlopen(req, timeout=60) as r:
            return r.status, json.loads(r.read().decode("utf-8", "replace") or "null")
    except urllib.error.HTTPError as e:
        try:
            return e.code, json.loads(e.read().decode("utf-8", "replace"))
        except Exception:
            return e.code, None


# ── el arbol: una fuente ya descubierta ENTERA, como la deja el Job ─────────
CAT = {
    "source": "ventas",
    "tables": [
        {"name": "public.clientes", "kind": "table", "primaryKey": ["id"],
         "columns": [{"name": "id", "type": "Int"}, {"name": "email", "type": "String"}]},
        {"name": "public.pedidos", "kind": "table", "primaryKey": ["id"],
         "columns": [{"name": "id", "type": "Int"}, {"name": "cliente_id", "type": "Int"}],
         "foreignKeys": [{"columns": ["cliente_id"], "references": "public.clientes", "toColumns": ["id"]}]},
        {"name": "legacy.viejo", "kind": "table", "primaryKey": ["id"],
         "columns": [{"name": "id", "type": "Int"}]},
    ],
}

d = tempfile.mkdtemp(prefix="base-elegida-")
subprocess.run([ORE, "init", "--name", "medida", "."], cwd=d, capture_output=True)
with open(os.path.join(d, "cat.json"), "w", encoding="utf-8") as f:
    json.dump(CAT, f)
subprocess.run(
    [ORE, "discover", "--from", "cat.json", "--out", "packages/ventas", "--name", "ventas"],
    cwd=d, capture_output=True,
)
assert os.path.isfile(os.path.join(d, "packages", "ventas", "discover.catalog.json"))

PUERTO = puerto_libre()
BASE = "http://127.0.0.1:%d" % PUERTO
srv = subprocess.Popen(
    [SERVE, "--repo", d, "--ore", ORE, "--bind", "127.0.0.1:%d" % PUERTO,
     "--identidad", "cabecera", "--no-es-produccion"],
    stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
)
try:
    for _ in range(50):
        try:
            urllib.request.urlopen(BASE + "/salud", timeout=1)
            break
        except Exception:
            time.sleep(0.1)
    else:
        raise SystemExit("ore-serve no arranco")

    # ── A · el nombre fisico ────────────────────────────────────────────────
    print()
    print("A - `/esquema` da el nombre FISICO de cada entidad")
    print()
    cod, e = pide("GET", "/paquetes/ventas/esquema")
    ents = {x["name"]: x for x in (e or {}).get("entities", [])}
    if cod == 200 and ents.get("Pedidos", {}).get("object") == "public.pedidos":
        bien("`Pedidos` dice `object: public.pedidos` — dos saltos resueltos")
    else:
        falla("`/esquema` %s: Pedidos = %r" % (cod, ents.get("Pedidos")))
    if ents.get("Viejo", {}).get("object") == "legacy.viejo":
        bien("y `Viejo` dice `legacy.viejo`: el esquema fisico viaja en el nombre")
    else:
        falla("Viejo = %r" % ents.get("Viejo"))

    # ── B · crear la base ───────────────────────────────────────────────────
    print()
    print("B - `POST /paquetes` induce con alcance del catalogo GUARDADO")
    print()
    cod, r = pide("POST", "/paquetes",
                  {"name": "analitica", "source": "ventas", "only": ["public.pedidos"]})
    if cod == 200:
        bien("200 · %s" % (r or {}).get("informe", "").split("\n")[0].strip())
    else:
        falla("POST devolvio %s: %r" % (cod, r))

    base = os.path.join(d, "packages", "analitica")
    ent = os.path.join(base, "entities")
    if os.path.isdir(ent) and sorted(os.listdir(ent)) == ["Pedidos.yaml"]:
        bien("`packages/analitica` tiene SOLO lo marcado")
    else:
        falla("entidades: %s" % (sorted(os.listdir(ent)) if os.path.isdir(ent) else "-"))

    alc = os.path.join(base, "discover.scope.json")
    if os.path.isfile(alc):
        a = json.load(open(alc, encoding="utf-8"))
        if a.get("only") == ["public.pedidos"] and a.get("source") == "ventas":
            bien("`discover.scope.json` escrito, con la fuente")
        else:
            falla("el alcance dice %r" % a)
    else:
        falla("no hay `discover.scope.json`")

    pedidos = open(os.path.join(ent, "Pedidos.yaml"), encoding="utf-8").read()
    if "analitica.Clientes" in pedidos:
        falla("la foranea hacia lo excluido quedo colgando")
    else:
        bien("la foranea hacia `clientes` no cuelga: se dice y no se emite")

    # ── C · la lista distingue ──────────────────────────────────────────────
    print()
    print("C - `/paquetes` distingue la BASE de la FUENTE entera")
    print()
    cod, l = pide("GET", "/paquetes")
    por = {p["name"]: p for p in (l or {}).get("packages", [])}
    if por.get("analitica", {}).get("scoped") is True and por["analitica"].get("source") == "ventas":
        bien("`analitica`: scoped=true, source=ventas")
    else:
        falla("analitica = %r" % por.get("analitica"))
    if por.get("ventas", {}).get("scoped") is False and por["ventas"].get("source") == "ventas":
        bien("`ventas`: scoped=false, source=ventas — es la fuente entera")
    else:
        falla("ventas = %r" % por.get("ventas"))

    # ── D · lo que se niega ─────────────────────────────────────────────────
    print()
    print("D - lo que se niega, se niega con motivo")
    print()
    casos = [
        ({"name": "analitica", "source": "ventas", "only": ["public.pedidos"]}, 409, "ya hay un paquete"),
        ({"name": "otra", "source": "ventas", "only": []}, 422, "`only` está vacío"),
        ({"name": "otra", "source": "nadie", "only": ["x"]}, 404, "no hay paquete"),
        ({"name": "otra", "source": "ventas", "only": ["public.pedidoss"]}, 422, "no tiene 1"),
        ({"name": "mal nombre", "source": "ventas", "only": ["public.pedidos"]}, 422, "`name`"),
    ]
    for cuerpo, esperado, pista in casos:
        cod, r = pide("POST", "/paquetes", cuerpo)
        dice = json.dumps(r, ensure_ascii=False) if r else ""
        if cod == esperado and pista in dice:
            bien("%s -> %d · %s" % (json.dumps(cuerpo, ensure_ascii=False)[:60], cod, pista))
        else:
            falla("%s -> %s (esperaba %d con «%s»): %s" % (cuerpo, cod, esperado, pista, dice[:120]))

    # Y una negativa no deja restos: el 422 de la errata no creo `packages/otra`.
    if os.path.exists(os.path.join(d, "packages", "otra")):
        falla("una negativa dejo `packages/otra` a medias")
    else:
        bien("una negativa no deja nada escrito")

finally:
    srv.kill()

print()
if fallos:
    print("== %d fallos ==" % len(fallos))
    sys.exit(1)
print("== todo verde ==")
