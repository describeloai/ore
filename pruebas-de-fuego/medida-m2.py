# -*- coding: utf-8 -*-
"""M2: «los 38 nombres que solo repiten; `properties` pasa a ANOTAR».

`entidad.md` lo cuenta como el peldano que adelgaza la entidad: 38 propiedades
que no dicen nada que la vista no diga, y que se irian. La premisa es que
REPITEN. Seis frentes para ver si es cierta:

  A. EL SUJETO      recontado hoy: cuantas solo tipan y cuantas anotan
  B. CUANTAS PUEDEN  de esas, cuantas NO sostienen otra cosa de la entidad
     IRSE            —una clave, una `via`, el origen de una derivada
  C. EL PRECIO      se van de verdad, y el esquema lo dice el motor
  D. DE DONDE SALDRIA  `physicalType` -> tipo OOS: cobertura, y si alcanza
  E. LA REGLA       `OOS2022` invertido
  F. VEREDICTO

Dos veces se equivoco esta medida antes de dar un numero, y las dos por lo
mismo: contar una cosa y simular otra. Primero quitaba el `type` en vez de la
propiedad; luego quitaba las 76 cuando la cuenta decia 22. Por eso `solo_tipan`
y `sostenidas` estan definidas UNA vez y las usan la cuenta y la simulacion.
"""
import collections
import pathlib
import re
import shutil
import subprocess
import tempfile

RAIZ = pathlib.Path(r"C:\ORE")
OOS = RAIZ / "vendor/oos"
EJEMPLO = OOS / "examples/acme-retail"
ORE = RAIZ / "target/debug/ore"

SPEC = ["backedBy", "implements", "nature", "principal", "primaryKey", "timeKey",
        "uniqueKeys", "temporal", "properties", "relations", "moved", "reserved"]


def correr(*args):
    s = subprocess.run([str(ORE), *map(str, args)], capture_output=True,
                       text=True, encoding="utf-8", errors="replace")
    return s.stdout + s.stderr


def docs(raiz):
    for f in sorted(raiz.rglob("*.yaml")):
        txt = f.read_text(encoding="utf-8", errors="replace")
        for d in re.split(r"^---\s*$", txt, flags=re.M):
            k = re.search(r"^kind:\s*(\w+)", d, re.M)
            if k:
                yield k.group(1), d, f


def seccion(d, clave, hermanas):
    if clave + ":" not in d:
        return ""
    resto = d.split(clave + ":", 1)[1]
    m = re.search(r"^  (?:%s):" % "|".join(hermanas), resto, re.M)
    return resto[: m.start()] if m else resto


def propiedades(d):
    """nombre -> el texto de esa propiedad, y nada del vecino."""
    cuerpo = seccion(d, "properties", SPEC)
    out, actual, nombre = {}, [], None
    for linea in cuerpo.split("\n"):
        m = re.match(r"^    ([A-Za-z_]\w*):(.*)$", linea)
        if m:
            if nombre:
                out[nombre] = "\n".join(actual)
            nombre, actual = m.group(1), [m.group(2)]
        elif nombre is not None and (linea.startswith("      ") or not linea.strip()):
            actual.append(linea)
        elif nombre:
            out[nombre] = "\n".join(actual)
            nombre, actual = None, []
    if nombre:
        out[nombre] = "\n".join(actual)
    return out


TODOS = list(docs(OOS)) + list(docs(RAIZ / "casos"))
print("== M2: `properties` pasa a anotar ==")

# -- A - EL SUJETO -----------------------------------------------------------
print()
print("A - EL SUJETO, recontado hoy")
solo_tipo, anotan, sin_tipo = [], 0, 0
tipos = collections.Counter()
for kind, d, f in TODOS:
    if kind != "Entity" or "backedBy:" not in d:
        continue
    for n, cuerpo in propiedades(d).items():
        claves = set(re.findall(r"(?:^|[{,\s])(\w+):", cuerpo))
        m = re.search(r"type:\s*([^,}\n]+)", cuerpo)
        claves.discard("type")
        if not m:
            sin_tipo += 1
        if claves:
            anotan += 1
        else:
            solo_tipo.append((n, m.group(1).strip() if m else "?"))
            tipos[m.group(1).strip() if m else "?"] += 1
print("   %-46s %3d" % ("propiedades que ANOTAN algo mas que el tipo", anotan))
print("   %-46s %3d" % ("propiedades que solo declaran `type`", len(solo_tipo)))
print("   %-46s %3d" % ("  ...y las que no declaran ni tipo", sin_tipo))
print()
print("   el reparto de tipos de las que «solo repiten»:")
for k, v in tipos.most_common(8):
    print("     %-24s %3d" % (k, v))

def solo_tipan(d):
    """Las propiedades que no dicen nada mas que su tipo."""
    return {n for n, c in propiedades(d).items()
            if not (set(re.findall(r"(?:^|[{,\s])(\w+):", c)) - {"type"})}


def sostenidas(d):
    """De esas, cuales estan NOMBRADAS por otra parte de la entidad.

    Una sola definicion, usada por la cuenta y por la simulacion. Tenerla dos
    veces era el fallo de la primera version: la cuenta decia «22 se pueden ir»
    y la simulacion quitaba las 76.
    """
    solo = solo_tipan(d)
    out = collections.defaultdict(set)
    for clave in ("primaryKey", "timeKey", "uniqueKeys"):
        for n in re.findall(r"[\[,\s](\w+)[\],]", seccion(d, clave, SPEC)[:200]):
            if n in solo:
                out[n].add(clave)
    for lista in re.findall(r"^      via:\s*\[([^\]]*)\]", d, re.M):
        for x in [y.strip() for y in lista.split(",")]:
            if x in solo:
                out[x].add("via")
    for lista in re.findall(r"derivedFrom:\s*\[([^\]]*)\]", d):
        for x in [y.strip().split(".")[-1] for y in lista.split(",")]:
            if x in solo:
                out[x].add("derivedFrom")
    return out


# -- B - CUANTAS SE PUEDEN IR DE VERDAD --------------------------------------
print()
print("B - DE ESAS %d, ¿CUANTAS SE PUEDEN IR? Las que no sostienen nada"
      % len(solo_tipo))
print("   Una propiedad que solo tipa puede estar NOMBRADA en otro sitio de la")
print("   propia entidad. Si se va, se lleva por delante lo que la nombra.")
sostienen = collections.Counter()
libres = 0
for kind, d, f in TODOS:
    if kind != "Entity" or "backedBy:" not in d:
        continue
    n_sost = sostenidas(d)
    for n in sorted(solo_tipan(d)):
        if n in n_sost:
            for k in n_sost[n]:
                sostienen[k] += 1
        else:
            libres += 1
for k, v in sostienen.most_common():
    print("     nombrada por %-14s %3d" % (k, v))
print("   %-46s %3d" % ("...y no las nombra nada: se pueden ir", libres))
print()
print("   -> M2 no son %d: son %d. Las otras %d son la clave primaria, la `via`"
      % (len(solo_tipo), libres, len(solo_tipo) - libres))
print("      de una relacion o el origen de una derivada, y quitarlas es lo")
print("      mismo que quitar `via`: borrar el dato, no cobrar una repeticion.")

# -- C - EL PRECIO, MEDIDO POR EL MOTOR --------------------------------------
print()
print("C - EL PRECIO: se van de verdad las que pueden, y decide el motor")
print("   `vista.rs::tipos_de_raiz`, en sus propias palabras:")
print("     «La vista NO TIPA —es fisica— asi que el tipo baja de LA ENTIDAD.")
print("      Lo que ninguna entidad nombra es `String`, que es lo unico que se")
print("      puede afirmar de una columna de la que solo se sabe el nombre.»")
print()
print("   O sea que quitar la propiedad no deja el campo sin tipo: lo deja en")
print("   `String`. Compila igual. Se mide cuantos cambian y a que.")


def esquemas(arbol):
    """vista -> {campo: tipo}. Si `ore view` fallo, se dice en vez de medir."""
    out = correr("view", arbol)
    if "error[" in out:
        return None, sorted(set(re.findall(r"error\[(OOS\d+)\]", out)))
    d = {}
    for bloque in out.split("\n\n"):
        v = bloque.split("\n", 1)[0].strip()
        m = re.search(r"^  esquema   (.*)$", bloque, re.M)
        if m and v:
            d[v] = dict(p.split(": ", 1) for p in m.group(1).split(" · ")
                        if ": " in p)
    return d, []


antes, err = esquemas(EJEMPLO)
assert antes is not None, "el ejemplo ya venia roto: %s" % err
tmp = pathlib.Path(tempfile.mkdtemp()) / "r"
shutil.copytree(EJEMPLO, tmp)
quitadas = []
for e in sorted(tmp.rglob("packages/*/entities/*.yaml")):
    t = e.read_text(encoding="utf-8")
    n_sost = sostenidas(t)
    for n in solo_tipan(t):
        if n in n_sost:
            continue
        # La forma breve —`n: { type: X }`— y la expandida con sus comentarios.
        nuevo = re.sub(r"^    %s:\s*\{[^}]*\}\n" % n, "", t, flags=re.M)
        if nuevo == t:
            nuevo = re.sub(r"^    %s:\n(?:      .*\n|\n(?=      ))*" % n, "", t,
                           flags=re.M)
        if nuevo != t:
            t = nuevo
            quitadas.append(n)
    e.write_text(t, encoding="utf-8")
print("   propiedades quitadas del ejemplo: %d (%s)"
      % (len(quitadas), ", ".join(sorted(quitadas))))
despues, err = esquemas(tmp)
shutil.rmtree(tmp.parent, ignore_errors=True)

if despues is None:
    print("   NO SE PUEDE MEDIR: quitarlas rompe el repo con %s" % ", ".join(err))
    print("   —y eso ya seria el resultado: no son propiedades que sobren.")
else:
    cambian = [(v, c, antes[v][c], despues.get(v, {}).get(c, "(se fue)"))
               for v in antes for c in antes[v]
               if despues.get(v, {}).get(c) != antes[v][c]]
    print("   %-46s %3d" % ("campos cuyo tipo CAMBIA", len(cambian)))
    for v, c, a, b in cambian:
        print("     %-32s %-16s -> %s" % ("%s.%s" % (v, c), a, b))
    if not cambian:
        print("     ninguno — todas las que se van eran `String` aqui, que es")
        print("     lo que el motor pone cuando nadie tipa")
    print()
    print("   -> y el modo de fallo es el que importa: NO ES UN ERROR. El repo")
    print("      sigue compilando, el campo pasa a `String` y nadie lo dice. Un")
    print("      tipo que se degrada en silencio es peor que uno que falta.")



# -- D - DE DONDE SALDRIA ----------------------------------------------------
print()
print("D - ¿DE DONDE SALDRIA EL TIPO DE LAS QUE SI SE VAN?")
cols = collections.Counter()
for kind, d, _ in TODOS:
    if kind != "Table" or "columns:" not in d:
        continue
    cuerpo = seccion(d, "columns", ["reads", "changes", "profile", "datasource", "object"])
    for linea in re.findall(r"^    [\"\w.]+:\s*(.*)$", cuerpo, re.M):
        cols["con" if "physicalType" in linea else "sin"] += 1
tot = cols["con"] + cols["sin"]
print("   columnas con `physicalType` : %3d de %3d (%d %%)"
      % (cols["con"], tot, 100 * cols["con"] // max(tot, 1)))
nucleo = (RAIZ / "crates/ore-core/src/types.rs").read_text(encoding="utf-8",
                                                           errors="replace")
lector = (RAIZ / "crates/ore-cli/src/lector.rs").read_text(encoding="utf-8",
                                                           errors="replace")
print("   mapeo `physicalType` -> tipo OOS en el NUCLEO : %s"
      % ("si" if "fn tipo_oos" in nucleo else "NO"))
print("   ...en el lector, una vez por driver           : %s"
      % ("si" if "fn tipo_oos" in lector else "no"))
no_triviales = [(n, t) for n, t in solo_tipo if t != "String"]
print()
print("   %d de las %d que solo tipan llevan un tipo que NO es `String`:"
      % (len(no_triviales), len(solo_tipo)))
for k, v in collections.Counter(t for _, t in no_triviales).most_common():
    print("     %-24s %2d" % (k, v))
print("   -> esas son las que se degradarian en silencio. Para salvarlas hace")
print("      falta una pieza que NO EXISTE: el mapeo del tipo fisico al tipo")
print("      OOS **en el nucleo** —hoy vive una vez por driver y solo en el")
print("      descubrimiento— y aun con ella quedaria un %d %% de columnas sin"
      % (100 * cols["sin"] // max(tot, 1)))
print("      `physicalType` del que no se puede sacar nada.")



# -- E - LA REGLA ------------------------------------------------------------
print()
print("E - LA REGLA QUE SE INVIERTE: `OOS2022`")
print("   hoy    cada PROPIEDAD debe ser campo de su vista, o declarar")
print("          `derivedFrom`. La entidad manda y la vista tiene que cubrirla.")
print("   con M2 cada ANOTACION debe nombrar un campo. La vista manda y la")
print("          entidad solo puede hablar de lo que hay.")
print("   -> es la misma comprobacion con los papeles cambiados: no hace falta")
print("      codigo nuevo. Esta parte de M2 se sostiene sola.")

# -- F - VEREDICTO -----------------------------------------------------------
print()
print("F - VEREDICTO")
print("   La premisa de M2 —«solo repiten»— es falsa a medias, y la mitad falsa")
print("   es la que decide:")
print()
print("     el NOMBRE si repite      lo dice `fields`, y `OOS2022` lo obliga")
print("     el TIPO no repite        no esta en ninguna vista, porque la vista")
print("                              es fisica y no tipa. Quitarlo no cobra una")
print("                              redundancia: BORRA EL DATO")
print()
print("   Es la misma forma exacta que tenia `via` en el §6 de `entidad.md`, y")
print("   el mismo motivo por el que `B0` no se pudo escribir: se contaba como")
print("   redundante algo que era la unica fuente.")
print()
print("   Lo que queda en pie de M2 es real y mas pequeño: la entidad no tiene")
print("   por que REPETIR EL NOMBRE de un campo para anotarlo. Eso es `OOS2022`")
print("   invertido, no hay pieza nueva que construir, y no toca el tipo.")
