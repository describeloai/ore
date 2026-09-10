# -*- coding: utf-8 -*-
"""El alcance de un `discover`: elegir que tablas entran, y que eso se sostenga.

Medido el 2026-09-10. La consola ya tiene el modal con las casillas —schema y
tabla, `CreateDatabaseModal.tsx`— y su seleccion muere en `useState`. Para que
llegue al arbol hacen falta tres cosas, y esta medida las ejerce:

  A. QUE SE PUEDA ELEGIR        `--only`, y `--only-file` cuando son cien
  B. QUE SOBREVIVA A `review`   volver a inducir no puede devolver lo excluido
  C. QUE `drift-detect` LO SEPA  o son 95 falsas alarmas en cada pasada

(C) es el motivo de que el alcance se ESCRIBA en vez de solo aplicarse, y tiene
tres casos que hay que distinguir. Los dos primeros son faciles de confundir y
el tercero es el que se pierde si uno se pasa de listo:

  en el alcance | estaba en el catalogo | que es
  --------------|-----------------------|--------------------------------
  si            | -                     | entra, se compara como siempre
  no            | si                    | lo miraron y dijeron que no
  no            | no                    | APARECIO DESPUES: nadie lo vio

La tercera fila es la que justifica que `discover.catalog.json` se guarde
entero. Sin ella, elegir cinco de cien deja la fuente ciega para siempre — que
es cambiar noventa y cinco falsas alarmas por una senal perdida.

Sin red y sin driver: `discover --from` acepta un catalogo escrito a mano.
"""
import json
import os
import shutil
import subprocess
import sys
import tempfile

# La consola de Windows es cp1252 y la salida de `ore` lleva flechas y comillas
# tipograficas. Sin esto, la medida se cae AL IMPRIMIR el resultado — que es el
# peor sitio para caerse: parece que fallo lo medido.
for f in (sys.stdout, sys.stderr):
    try:
        f.reconfigure(errors="replace")
    except AttributeError:
        pass

ORE = os.path.join(os.environ.get("ORE_RAIZ", r"C:\ORE"), "target", "release", "ore.exe")
if not os.path.exists(ORE):
    ORE = shutil.which("ore") or ORE

fallos = []


def corre(args, cwd):
    p = subprocess.run(
        [ORE] + args, cwd=cwd, capture_output=True, text=True, encoding="utf-8", errors="replace"
    )
    return p.returncode, (p.stdout or "") + (p.stderr or "")


def falla(que):
    fallos.append(que)
    print("  x  %s" % que)


def bien(que):
    print("  ok %s" % que)


def tabla(nombre, foranea=None):
    t = {
        "name": nombre,
        "kind": "table",
        "primaryKey": ["id"],
        "columns": [{"name": "id", "type": "Int"}, {"name": "dato", "type": "String"}],
    }
    if foranea:
        t["columns"].append({"name": "otro_id", "type": "Int"})
        t["foreignKeys"] = [{"columns": ["otro_id"], "references": foranea, "toColumns": ["id"]}]
    return t


CAT = {
    "source": "ventas",
    "tables": [tabla("public.clientes"), tabla("public.pedidos", foranea="public.clientes")],
}


def taller(catalogo=CAT):
    d = tempfile.mkdtemp(prefix="alcance-")
    corre(["init", "--name", "medida", "."], d)
    with open(os.path.join(d, "cat.json"), "w", encoding="utf-8") as f:
        json.dump(catalogo, f)
    return d


# ── A · elegir ──────────────────────────────────────────────────────────────
print()
print("A - `--only` recorta, y lo ESCRIBE")
print()

d = taller()
codigo, salida = corre(
    ["discover", "--from", "cat.json", "--out", "packages/p", "--name", "medida",
     "--only", "public.pedidos"],
    d,
)
ent = os.path.join(d, "packages", "p", "entities")
emitidas = sorted(os.listdir(ent)) if os.path.isdir(ent) else []
if emitidas == ["Pedidos.yaml"]:
    bien("entra solo lo pedido: %s" % ", ".join(emitidas))
else:
    falla("entraron %s" % (emitidas or "ninguna"))

ruta = os.path.join(d, "packages", "p", "discover.scope.json")
if not os.path.exists(ruta):
    falla("no escribio `discover.scope.json`")
else:
    escrito = json.load(open(ruta, encoding="utf-8"))
    if escrito.get("only") == ["public.pedidos"] and escrito.get("source") == "ventas":
        bien("`discover.scope.json` dice que se llevo y de que fuente")
    else:
        falla("el alcance escrito dice %r" % escrito)

# El catalogo se guarda ENTERO: es el que permite la tercera fila de la tabla.
guardado = json.load(open(os.path.join(d, "packages", "p", "discover.catalog.json"), encoding="utf-8"))
if len(guardado["tables"]) == 2:
    bien("`discover.catalog.json` se guarda entero, sin recortar")
else:
    falla("el catalogo guardado salio recortado a %d tablas" % len(guardado["tables"]))

# Y la foranea hacia lo que no entra no cuelga: se dice y no se emite.
pedidos = open(os.path.join(ent, "Pedidos.yaml"), encoding="utf-8").read()
if "medida.Clientes" in pedidos:
    falla("`Pedidos` apunta a `medida.Clientes`, que no entra")
elif "public.clientes" in pedidos:
    bien("la foranea hacia lo excluido se dice, y no se emite")
else:
    falla("la foranea hacia lo excluido desaparecio en silencio")

# Una errata no se traga: se acaba de teclear.
codigo, salida = corre(
    ["discover", "--from", "cat.json", "--out", "packages/q", "--name", "q",
     "--only", "public.pedidoss"],
    d,
)
if codigo == 65 and "public.pedidos`" in salida:
    bien("una errata para el mando, y dice como se llaman de verdad")
else:
    falla("la errata salio con %s: %s" % (codigo, salida.strip()[:120]))

# ── A2 · cien tablas no caben en una linea de ordenes ───────────────────────
d2 = taller()
with open(os.path.join(d2, "lista.txt"), "w", encoding="utf-8") as f:
    f.write("# las que quiero\npublic.pedidos\n\n")
codigo, salida = corre(
    ["discover", "--from", "cat.json", "--out", "packages/p", "--name", "medida",
     "--only-file", "lista.txt"],
    d2,
)
ent2 = os.path.join(d2, "packages", "p", "entities")
if sorted(os.listdir(ent2)) == ["Pedidos.yaml"]:
    bien("`--only-file` hace lo mismo, y admite anotaciones")
else:
    falla("`--only-file` dio %s" % sorted(os.listdir(ent2)))

# ── B · sobrevivir a `review` ───────────────────────────────────────────────
print()
print("B - volver a inducir NO devuelve lo excluido")
print()

with open(os.path.join(d, "resp.yaml"), "w", encoding="utf-8") as f:
    f.write('answers:\n  dueno/medida: "team:datos"\n')
codigo, salida = corre(["review", "packages/p", "--answers", "resp.yaml"], d)
emitidas = sorted(os.listdir(ent))
if emitidas == ["Pedidos.yaml"]:
    bien("`review` respeta el alcance: sigue habiendo una entidad")
else:
    falla("`review` devolvio %s" % ", ".join(emitidas))

codigo, salida = corre(["validate", "packages/p"], d)
if "OOS2005" in salida:
    falla("`validate` dice OOS2005: quedo una relacion colgada")
else:
    bien("`validate` no encuentra relaciones colgadas")

# Un alcance de otra fuente no se ignora: recortaria a cero.
with open(ruta, "w", encoding="utf-8") as f:
    json.dump({"source": "compras", "only": ["public.pedidos"]}, f)
codigo, salida = corre(["review", "packages/p"], d)
if codigo == 65 and "compras" in salida:
    bien("un alcance de otra fuente se niega, en vez de recortar a cero")
else:
    falla("el alcance ajeno salio con %s" % codigo)
with open(ruta, "w", encoding="utf-8") as f:
    json.dump({"source": "ventas", "only": ["public.pedidos"]}, f)

# ── C · las tres filas de la tabla ──────────────────────────────────────────
print()
print("C - `drift-detect` distingue «no lo elegi» de «no lo vi»")
print()

# El origen de HOY: lo de antes mas una tabla que no existia al elegir.
luego = {"source": "ventas", "tables": CAT["tables"] + [tabla("public.recien_llegada")]}
with open(os.path.join(d, "luego.json"), "w", encoding="utf-8") as f:
    json.dump(luego, f)

codigo, salida = corre(["drift-detect", "--from", "luego.json", "--path", "packages/p"], d)
print("     %s" % salida.strip().replace("\n", "\n     "))

if "fuera del alcance declarado" in salida:
    bien("lo descartado se dice, contado, y no es deriva")
else:
    falla("no dijo nada de lo que quedo fuera del alcance")

if "public.clientes" in salida:
    falla("`public.clientes` sale como deriva y alguien dijo que no entraba")
else:
    bien("`public.clientes` no sale como deriva: fue una decision")

if "public.recien_llegada" in salida and "después del alcance" in salida:
    bien("`public.recien_llegada` SI sale, y dice que aparecio despues")
else:
    falla("la tabla nueva no sale, o no dice por que — es la senal que importa")

if codigo == 2:
    bien("y sale con 2, que es lo que un pipeline mira")
else:
    falla("salio con %s y hay deriva de verdad" % codigo)

# Y sin la tabla nueva, cero deriva y cero.
codigo, salida = corre(["drift-detect", "--from", "cat.json", "--path", "packages/p"], d)
if codigo == 0 and "sin deriva" in salida:
    bien("contra el origen de entonces: sin deriva, y sale con 0")
else:
    falla("el origen sin cambios dio %s: %s" % (codigo, salida.strip()[:120]))

print()
if fallos:
    print("== %d fallos ==" % len(fallos))
    sys.exit(1)
print("== todo verde ==")
