# -*- coding: utf-8 -*-
"""`ore package split`: la clausura, y si de verdad es pequena.

El argumento para el verbo era este: mover un documento solo deja referencias
cruzando, y hay un CONJUNTO que va junto. Se vio en vivo —mover una tabla dejo
su vista atras y un `OOS2028`— pero de un caso no sale una regla.

La pregunta que decide el verbo entero: ¿la clausura de un documento es PEQUENA,
o es el paquete entero? Si es pequena, `split` calcula algo que una persona no
puede; si es todo, `split` no calcula una clausura — ELIGE UN CORTE, y eso es
otro mando con otra conversacion.

  A. EL GRAFO                  que referencia se cuenta, y por que
  B. LAS COMPONENTES           sobre dos arboles reales: descubierto y curado
  C. EL COSTE DE UN CORTE      cruces si mueves uno, si mueves su clausura
  D. LOS CRITERIOS             cuales son derivables y cuales una pregunta
  E. LO QUE `split` NO DECIDE
"""
import os
import pathlib
import re
import textwrap
from collections import defaultdict

DESCUBIERTO = pathlib.Path(
    os.environ["TEMP"]
) / "claude/C--ORE/7341bcd2-5b71-4a08-b225-8a42c2c65a59/scratchpad/split/packages"
CURADO = pathlib.Path(r"C:\ORE\vendor\oos\examples\acme-retail\packages")


def parrafo(t, sangria="     ", ancho=68):
    for l in textwrap.wrap(t, ancho):
        print("%s%s" % (sangria, l))


# Las claves con las que un documento nombra a otro. Son las mismas que
# `exporta::referencias` enumera; aqui se leen por texto porque esta medida no
# tiene que compilar nada.
CLAVES = ["backedBy", "table", "view", "target", "is", "implements", "derivedFrom"]


def documentos(raiz):
    """qname -> (kind, texto), por paquete."""
    out = {}
    for f in raiz.rglob("*.yaml"):
        t = f.read_text(encoding="utf-8", errors="replace")
        k = re.search(r"^kind:\s*(\w+)", t, re.M)
        n = re.search(r"name:\s*([A-Za-z0-9_]+)", t)
        ns = re.search(r"namespace:\s*([A-Za-z0-9_]+)", t)
        if not k or not n or k.group(1) == "Package":
            continue
        qn = "%s.%s" % (ns.group(1), n.group(1)) if ns else n.group(1)
        paq = f.parent.parent.name
        out[qn] = (k.group(1), t, paq)
    return out


def aristas(docs):
    """Quien nombra a quien, SIN direccion: para el corte da igual el sentido."""
    g = defaultdict(set)
    cortos = {}
    for qn in docs:
        cortos.setdefault(qn.split(".")[-1], []).append(qn)
    for qn, (_, t, _) in docs.items():
        mio = qn.rsplit(".", 1)[0] if "." in qn else None
        for clave in CLAVES:
            for m in re.finditer(r"\b%s:\s*\[?([A-Za-z0-9_.\s,]+?)\]?\s*[},\n]" % clave, t):
                for ref in m.group(1).split(","):
                    ref = ref.strip()
                    if not ref:
                        continue
                    destino = None
                    if ref in docs:
                        destino = ref
                    elif mio and "%s.%s" % (mio, ref) in docs:
                        destino = "%s.%s" % (mio, ref)
                    elif len(cortos.get(ref, [])) == 1:
                        destino = cortos[ref][0]
                    if destino and destino != qn:
                        g[qn].add(destino)
                        g[destino].add(qn)
    return g


def componentes(docs, g, paquete):
    dentro = [q for q, (_, _, p) in docs.items() if p == paquete]
    visto, out = set(), []
    for q in dentro:
        if q in visto:
            continue
        pila, comp = [q], []
        while pila:
            x = pila.pop()
            if x in visto or docs.get(x, (None, None, None))[2] != paquete:
                continue
            visto.add(x)
            comp.append(x)
            pila.extend(g.get(x, ()))
        out.append(sorted(comp))
    return sorted(out, key=len, reverse=True)


print("== el corte de un paquete, medido ==")

# -- A -----------------------------------------------------------------------
print()
print("A - EL GRAFO: que arista se cuenta")
print()
parrafo("Una arista es «este documento NOMBRA a este otro»: `backedBy`, "
        "`from.table`, `from.view`, `target` de una relacion, `is`, "
        "`implements`, `derivedFrom`. Las mismas que `exporta::referencias` "
        "enumera.")
print()
parrafo("Y se cuenta SIN DIRECCION, que es lo que costo entender: mover un "
        "documento no rompe solo a quien lo nombra —tambien hace cruzar lo que "
        "EL nombra—. Para el corte da igual el sentido: una arista que cruza el "
        "limite es una arista que cruza.")

# -- B -----------------------------------------------------------------------
print()
print("B - LAS COMPONENTES, sobre dos arboles reales")
print()
for etiqueta, raiz in [("DESCUBIERTO · `ore discover` de BigQuery", DESCUBIERTO),
                       ("CURADO · `examples/acme-retail`", CURADO)]:
    if not raiz.exists():
        print("   %s: no esta" % etiqueta)
        continue
    docs = documentos(raiz)
    g = aristas(docs)
    paquetes = sorted({p for _, _, p in docs.values()})
    print("   %s" % etiqueta)
    for paq in paquetes:
        comps = componentes(docs, g, paq)
        n = sum(len(c) for c in comps)
        tams = [len(c) for c in comps]
        print("     %-12s %2d documentos, %2d componente(s)  tamanos %s"
              % (paq, n, len(comps), tams))
    print()

# -- C -----------------------------------------------------------------------
print()
print("C - EL COSTE DE UN CORTE")
print()
docs = documentos(DESCUBIERTO)
g = aristas(docs)
paq = "ventas"
comps = componentes(docs, g, paq)
if comps:
    c = comps[0]
    uno = c[0]
    cruces_uno = len([v for v in g.get(uno, ()) if v != uno and v in docs])
    print("   Si se mueve UNO solo:   `%s`" % uno)
    print("     referencias que pasan a cruzar: %d" % cruces_uno)
    print()
    print("   Si se mueve SU COMPONENTE (%d documentos):" % len(c))
    fuera = 0
    for q in c:
        for v in g.get(q, ()):
            if v not in c:
                fuera += 1
    print("     referencias que pasan a cruzar: %d" % fuera)
    print()
    parrafo("Y ahi esta la respuesta al verbo: si la componente es pequena y "
            "sale con CERO cruces, `split` calcula algo que una persona no "
            "puede calcular bien. Si la componente es el paquete entero, no hay "
            "clausura que calcular — hay que ELEGIR UN CORTE, y el mando pasa "
            "de calcular a preguntar.")

print()
print("   Y para un paquete que es UNA sola componente, el corte mas barato:")
print()
import itertools
for paq, raiz in [("hr", CURADO), ("supply", CURADO), ("customers", CURADO)]:
    d2 = documentos(raiz)
    g2 = aristas(d2)
    comps2 = componentes(d2, g2, paq)
    if len(comps2) != 1 or len(comps2[0]) < 2:
        continue
    c2 = comps2[0]
    mejor, como = None, None
    for k in range(1, len(c2)):
        for sub in itertools.combinations(c2, k):
            s = set(sub)
            cruces = sum(1 for q in s for v in g2.get(q, ()) if v in c2 and v not in s)
            if mejor is None or cruces < mejor:
                mejor, como = cruces, sorted(s)
    print("     %-10s %d documentos · corte mas barato: %d cruce(s)"
          % (paq, len(c2), mejor))
    print("                  %s" % " + ".join(x.split(".")[-1] for x in como))
print()
parrafo("Asi que en un paquete modelado NO hay corte gratis: el mas barato "
        "cuesta uno. Y eso no lo convierte en un mal corte —puede ser "
        "exactamente el limite que se queria trazar— pero si cambia lo que el "
        "mando tiene que hacer: DECIR EL PRECIO, no buscarlo.")

# -- D -----------------------------------------------------------------------
print()
print("D - LOS CRITERIOS DE CORTE, y cuales son derivables")
print()
CRIT = [
    ("por componente", "DERIVABLE", "no hace falta preguntar nada: son las "
     "piezas que ya estan separadas. Es el unico corte con coste CERO"),
    ("por `datasource`", "DERIVABLE", "las tablas lo declaran, y la vista y la "
     "entidad cuelgan de ellas. Es el corte natural de un paquete descubierto"),
    ("por lista explicita", "SE PIDE", "y entonces `split` calcula la clausura "
     "y dice que arrastra — que es el aviso que importa"),
    ("por dueno", "NO EXISTE", "`owner` es del PAQUETE, no del documento. Una "
     "vista declara el suyo, una tabla no: no hay criterio uniforme"),
]
print("   %-22s %-12s %s" % ("criterio", "que es", ""))
print("   " + "-" * 70)
for q, tipo, _ in CRIT:
    print("   %-22s %-12s" % (q, tipo))
print()
for q, _, por in CRIT:
    print("   · %s" % q)
    parrafo(por, "       ")
    print()

# -- E -----------------------------------------------------------------------
print()
print("E - LO QUE `split` NO PUEDE DECIDIR")
print()
for q in [
    "EL DUENO DEL PAQUETE NUEVO. Es un acto de gobierno, y por eso `package "
    "new` es un verbo aparte: `split` lo llama o lo exige, no lo inventa",
    "SI EL CRUCE ES ACEPTABLE. Un corte con tres cruces no es peor que uno con "
    "cero: puede ser exactamente el limite que se queria trazar. Lo que `split` "
    "debe hacer es DECIR el coste antes de mover, no minimizarlo por su cuenta",
    "Y `exports`. Es «esto lo expongo a proposito», y ya se decidio en `move`: "
    "se dice la linea, no se escribe",
]:
    parrafo("· " + q, "     ")
    print()
