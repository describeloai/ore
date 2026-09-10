# -*- coding: utf-8 -*-
"""Una foranea hacia una tabla que NO se emite: `review` la cerraba en verde.

Medido el 2026-09-10, al preguntar si el alcance de un `discover` se puede
aplicar filtrando el catalogo. La respuesta corta es que no se podia, y el
motivo resulto ser un fallo que ya estaba y que nada tenia que ver con el
alcance:

    entities/Pedidos.yaml    target: hoy.Clientes
    entities/Clientes.yaml   <- retirado por `review` tres lineas antes

`inductor.rs::entidad_yaml` derivaba el destino con `entidad(&f.destino)` —del
NOMBRE DE LA TABLA— en vez de mirar `nombres`, que es el mapa de lo que
realmente se emite. Dos consecuencias, y la segunda es la peor:

  A. El destino puede NO EXISTIR      `omitir` sobre la tabla apuntada
  B. El destino puede ser OTRO        una colision resuelta con otro nombre
                                      da un nombre derivado que SI existe y
                                      es la entidad de al lado

(A) lo caza `ore validate` con OOS2005, asi que es ruidoso: `review` cierra en
verde y el compilador te para despues. (B) NO LO CAZA NADIE cuando el nombre
derivado resulta ser el de otra entidad emitida — que es exactamente lo que
pasa al resolver una colision dejando el nombre corto a la hermana. El
documento valida, tiene la aridad correcta, los tipos casan, y enlaza con la
tabla de al lado.

  colision/Cliente:
    public.cliente: ClienteActivo     <- el destino de la foranea
    legacy.cliente: Cliente           <- y `entidad("public.cliente")` dice ESTA

Por eso la medida usa esa reparticion y no otra: con cualquier otra, OOS2005
tapa el fallo y la prueba pasaria por el motivo equivocado.

La medida corre los dos casos contra el binario y comprueba que ninguno
sobrevive. No usa red ni driver: `discover --from` acepta un catalogo escrito
a mano, que es justo lo que lo hace ejercitable.
"""
import json
import os
import shutil
import subprocess
import sys
import tempfile

ORE = os.path.join(os.environ.get("ORE_RAIZ", r"C:\ORE"), "target", "release", "ore.exe")
if not os.path.exists(ORE):
    ORE = shutil.which("ore") or ORE

fallos = []


def corre(args, cwd):
    p = subprocess.run(
        [ORE] + args, cwd=cwd, capture_output=True, text=True, encoding="utf-8", errors="replace"
    )
    return p.returncode, (p.stdout or "") + (p.stderr or "")


def arbol():
    d = tempfile.mkdtemp(prefix="colgada-")
    corre(["init", "--name", "medida", "."], d)
    return d


def falla(que):
    fallos.append(que)
    print("  x  %s" % que)


def bien(que):
    print("  ok %s" % que)


# ── A · el destino no se emite ──────────────────────────────────────────────
#
# `rows: 0` es lo que abre la decision `filas/`, y `omitir` es una de sus dos
# respuestas. Es la via mas corta para sacar una tabla del paquete sin tocar
# nada mas.
print()
print("A - una foranea hacia una tabla OMITIDA")
print()

CAT_A = {
    "source": "ventas",
    "tables": [
        {
            "name": "public.clientes",
            "kind": "table",
            "rows": "0",
            "primaryKey": ["id"],
            "columns": [{"name": "id", "type": "Int"}, {"name": "email", "type": "String"}],
        },
        {
            "name": "public.pedidos",
            "kind": "table",
            "primaryKey": ["id"],
            "columns": [{"name": "id", "type": "Int"}, {"name": "cliente_id", "type": "Int"}],
            "foreignKeys": [
                {"columns": ["cliente_id"], "references": "public.clientes", "toColumns": ["id"]}
            ],
        },
    ],
}

d = arbol()
with open(os.path.join(d, "cat.json"), "w", encoding="utf-8") as f:
    json.dump(CAT_A, f)
corre(["discover", "--from", "cat.json", "--out", "packages/p", "--name", "medida"], d)
with open(os.path.join(d, "resp.yaml"), "w", encoding="utf-8") as f:
    f.write('answers:\n  filas/public.clientes: omitir\n  dueno/medida: "team:datos"\n')
_, salida = corre(["review", "packages/p", "--answers", "resp.yaml"], d)

ent = os.path.join(d, "packages", "p", "entities")
if os.path.exists(os.path.join(ent, "Clientes.yaml")):
    falla("`Clientes` sigue emitida: la medida no ejerce lo que dice ejercer")
else:
    bien("`Clientes` retirada, que es lo que se pidio")

pedidos = open(os.path.join(ent, "Pedidos.yaml"), encoding="utf-8").read()
if "medida.Clientes" in pedidos:
    falla("`Pedidos` apunta a `medida.Clientes`, que no existe")
else:
    bien("`Pedidos` no apunta a lo que no existe")

# Y no en silencio: la relacion que se cae se dice.
if "public.clientes" in pedidos:
    bien("y lo DICE: la foranea caida deja constancia en el documento")
else:
    falla("la foranea desaparecio en silencio")

codigo, salida = corre(["validate", "packages/p"], d)
if "OOS2005" in salida:
    falla("`validate` dice OOS2005: el paquete que `review` cerro en verde no valida")
else:
    bien("`validate` no dice OOS2005")

# ── B · el destino existe pero se llama de otra forma ───────────────────────
#
# Dos tablas que dan el mismo identificador de OOS: `public.cliente` y
# `ventas.cliente` son ambas `Cliente`. Al resolver la colision, una se queda
# con otro nombre — y `entidad("public.cliente")` sigue diciendo `Cliente`.
print()
print("B - el destino resolvio una COLISION y el nombre derivado es OTRA entidad")
print()

CAT_B = {
    "source": "ventas",
    "tables": [
        {
            "name": "public.cliente",
            "kind": "table",
            "primaryKey": ["id"],
            "columns": [{"name": "id", "type": "Int"}, {"name": "email", "type": "String"}],
        },
        {
            "name": "legacy.cliente",
            "kind": "table",
            "primaryKey": ["id"],
            "columns": [{"name": "id", "type": "Int"}, {"name": "nif", "type": "String"}],
        },
        {
            "name": "public.pedido",
            "kind": "table",
            "primaryKey": ["id"],
            "columns": [{"name": "id", "type": "Int"}, {"name": "cliente_id", "type": "Int"}],
            "foreignKeys": [
                {"columns": ["cliente_id"], "references": "public.cliente", "toColumns": ["id"]}
            ],
        },
    ],
}

d = arbol()
with open(os.path.join(d, "cat.json"), "w", encoding="utf-8") as f:
    json.dump(CAT_B, f)
_, salida = corre(["discover", "--from", "cat.json", "--out", "packages/p", "--name", "medida"], d)
if "colision" not in salida.lower():
    print("  ..  aviso: no salio decision de colision; la medida B no ejerce nada")
    print(salida[:600])

with open(os.path.join(d, "resp.yaml"), "w", encoding="utf-8") as f:
    f.write(
        "answers:\n"
        # ⛔ El reparto NO es arbitrario. `legacy.cliente` se queda el nombre
        #    CORTO, que es el que `entidad("public.cliente")` deriva. Asi el
        #    destino equivocado EXISTE y el fallo es silencioso.
        "  colision/Cliente:\n"
        "    public.cliente: ClienteActivo\n"
        "    legacy.cliente: Cliente\n"
        '  dueno/medida: "team:datos"\n'
    )
_, salida = corre(["review", "packages/p", "--answers", "resp.yaml"], d)

ent = os.path.join(d, "packages", "p", "entities")
emitidas = sorted(os.listdir(ent)) if os.path.exists(ent) else []
print("     entidades: %s" % ", ".join(emitidas))

pedido = [x for x in emitidas if x.lower().startswith("pedido")]
if not pedido:
    falla("no se emitio la entidad del pedido: la medida B no llego a ejercer nada")
else:
    txt = open(os.path.join(ent, pedido[0]), encoding="utf-8").read()
    destino = [l.strip() for l in txt.splitlines() if "target:" in l]
    print("     apunta a: %s" % (destino or "-"))
    if any(l.split()[-1] == "medida.Cliente" for l in destino):
        falla("apunta a `medida.Cliente` — la entidad de la HERMANA, y valida")
    elif any("ClienteActivo" in l for l in destino):
        bien("apunta a `ClienteActivo`, el nombre que la colision decidio")
    elif not destino:
        falla("no emitio la arista: el destino SI se emite, solo se llama de otra forma")

    # Y aqui `validate` no sirve de red: las dos entidades existen, asi que el
    # enlace equivocado pasa. Por eso la comprobacion de arriba mira el nombre
    # y no el codigo de salida.
    codigo, salida = corre(["validate", "packages/p"], d)
    if "OOS2005" in salida:
        falla("`validate` dice OOS2005: el reparto no ejercio el caso silencioso")
    else:
        bien("`validate` calla — que es justo por lo que este caso necesitaba medirse")

print()
if fallos:
    print("== %d fallos ==" % len(fallos))
    sys.exit(1)
print("== todo verde ==")
