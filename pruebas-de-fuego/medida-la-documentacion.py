# -*- coding: utf-8 -*-
"""¿Dice la documentacion el modelo de hoy, y lo dice UNA vez?

Cinco peldanos de medidas han cambiado el modelo por debajo de los documentos.
Tres de ellos terminaron descartando lo que iban a hacer, asi que lo que queda
escrito es, en parte, el plan y no el resultado. Cuatro frentes:

  A. EL INVENTARIO   que documentos hay, cuanto ocupan y que dice cada uno
                     que es. Un documento sin proposito declarado es un sitio
                     donde algo se repite
  B. LO REPETIDO     frases largas que aparecen en dos o mas sitios. No busca
                     plagio: busca DONDE VIVE cada idea, que es lo que hay que
                     decidir para que viva en uno
  C. LOS NUMEROS     toda cifra que la documentacion afirma, y si el arbol la
                     sigue sosteniendo. Las que no, son claims caducados
  D. LAS PROMESAS    lo que se dice «pendiente», «sin medir» o «en cola», que
                     es donde se acumula lo que ya paso
"""
import collections
import pathlib
import re
import subprocess

RAIZ = pathlib.Path(r"C:\ORE")
DOCS = RAIZ / "docs"
OOS = RAIZ / "vendor/oos"


def texto(f):
    return f.read_text(encoding="utf-8", errors="replace")


def frases(t):
    """Frases de prosa, sin codigo ni tablas: es donde viven los claims."""
    t = re.sub(r"```[\s\S]*?```", " ", t)
    t = re.sub(r"^\|.*$", " ", t, flags=re.M)
    for f in re.split(r"(?<=[.:])\s+", t):
        f = " ".join(f.split())
        if len(f) > 60:
            yield f


FICHEROS = sorted(DOCS.glob("*.md")) + sorted((DOCS / "decisions").glob("*.md"))
print("== la documentacion, medida ==")

# -- A - EL INVENTARIO -------------------------------------------------------
print()
print("A - EL INVENTARIO: que hay, y que dice cada uno que es")
for f in sorted(DOCS.glob("*.md")):
    t = texto(f)
    lineas = t.count("\n")
    # La primera frase de prosa detras del titulo: el proposito declarado.
    cuerpo = t.split("\n", 1)[1] if "\n" in t else ""
    prop = next((x for x in frases(cuerpo)), "(sin proposito declarado)")
    print("   %-30s %4d lineas" % (f.name, lineas))
    print("      %s" % (prop[:96] + ("..." if len(prop) > 96 else "")))

# -- B - LO REPETIDO ---------------------------------------------------------
print()
print("B - LO REPETIDO: la misma frase en dos o mas documentos")
donde = collections.defaultdict(set)
for f in FICHEROS:
    for fr in frases(texto(f)):
        # Normalizada: sin comillas ni enfasis, que es donde varia la copia.
        n = re.sub(r"[`*_«»\"'—–-]", "", fr.lower())
        n = " ".join(n.split())
        donde[n].add(f.name)
repes = {k: v for k, v in donde.items() if len(v) > 1}
print("   frases de prosa distintas   : %d" % len(donde))
print("   ...en DOS o mas documentos  : %d" % len(repes))
for k, v in sorted(repes.items(), key=lambda x: -len(x[0]))[:8]:
    print("     [%s]" % ", ".join(sorted(v)))
    print("       %s" % (k[:100] + ("..." if len(k) > 100 else "")))

# Y lo que importa mas que la frase: EL CONCEPTO. Donde se define cada pieza.
print()
print("   Y por concepto — cuantos documentos lo DEFINEN, no lo mencionan:")
DEFINE = {
    "la tabla es un hecho": r"tabla es un hecho|tabla.{0,20}es un.{0,10}hecho",
    "la vista es una pregunta": r"vista es (una )?pregunta",
    "la vista no lleva significado": r"vista no lleva significado|vista no tipa|no clasifica",
    "la copia es la respuesta": r"copia es la respuesta|par materializado",
    "raiz de lectura": r"ra[ií]z de lectura",
    "P2 lo derivable no se declara": r"derivable no se declara",
    "P4 omitir es cerrar": r"omitir.{0,20}no es.{0,20}abrir|omitir un conducto|denegaci[oó]n por defecto",
}
for nombre, pat in DEFINE.items():
    fs = [f.name for f in FICHEROS if re.search(pat, texto(f), re.I)]
    marca = "  <-- disperso" if len(fs) > 3 else ""
    print("     %-32s %d  %s%s" % (nombre, len(fs), ", ".join(fs[:4]), marca))

# -- C - LOS NUMEROS ---------------------------------------------------------
print()
print("C - LOS NUMEROS que la documentacion afirma, y si el arbol los sostiene")


def contar(patron, donde, extra=None):
    cmd = ["grep", "-rl", patron, str(donde)] + (extra or [])
    return len(subprocess.run(cmd, capture_output=True, text=True).stdout.split())


def entidades():
    n = con_bb = 0
    for f in list(OOS.rglob("*.yaml")) + list((RAIZ / "casos").rglob("*.yaml")):
        t = texto(f)
        for d in re.split(r"^---\s*$", t, flags=re.M):
            if re.search(r"^kind:\s*Entity", d, re.M):
                n += 1
                if re.search(r"^  backedBy:", d, re.M):
                    con_bb += 1
    return n, con_bb


n_ent, n_bb = entidades()
HECHOS = {
    "entidades en el arbol": n_ent,
    "...con `backedBy`": n_bb,
    "...sin el": n_ent - n_bb,
    "vistas": contar("kind: View", OOS, ["--include=*.yaml"])
    + contar("kind: View", RAIZ / "casos", ["--include=*.yaml"]),
    "tablas": contar("kind: Table", OOS, ["--include=*.yaml"]),
}
for k, v in HECHOS.items():
    print("   %-28s %3d" % (k, v))
print()
cifras = collections.Counter()
for f in FICHEROS:
    for m in re.finditer(r"\b(\d{1,3}) de (\d{1,3})\b", texto(f)):
        cifras[(m.group(0), f.name)] += 1
print("   claims de la forma «N de M» en la documentacion: %d" % len(cifras))
vivos = set(str(v) for v in HECHOS.values())
for (c, f), _ in sorted(cifras.items()):
    a, b = re.match(r"(\d+) de (\d+)", c).groups()
    ok = b in vivos or a in vivos
    print("     %-14s %-26s %s" % (c, f, "" if ok else "<-- comprobar"))

# -- D - LAS PROMESAS --------------------------------------------------------
print()
print("D - LAS PROMESAS: lo que se dice pendiente, y puede que ya no lo este")
PEND = r"sin medir|pendiente|en cola|queda abierto|no se decide aqu|falta medir|hay que decidir"
for f in FICHEROS:
    t = texto(f)
    hits = [" ".join(x.split())[:88] for x in
            re.findall(r"[^\n.]{0,80}(?:%s)[^\n.]{0,40}" % PEND, t, re.I)]
    if hits:
        print("   %s — %d" % (f.name, len(hits)))
        for h in hits[:4]:
            print("       %s" % h)
