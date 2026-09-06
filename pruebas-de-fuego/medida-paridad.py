# -*- coding: utf-8 -*-
"""El censo de paridad: que regla puede saltar en cada paradigma, y donde se
prueba.

`Binding` no es legacy: v1alpha1 no caduca, y un paquete firmado hace dos anos
tiene que seguir compilando o la firma no vale nada. Asi que no hay una
migracion que hacer — hay DOS PARADIGMAS VIVOS, y el riesgo no es que quede
codigo viejo: es que una regla valga en los dos y solo se pruebe en uno.

Ese es el patron que faltaba, y aqui se mide antes de decidir si se escribe
como guardia. Cuatro frentes:

  A. DONDE PUEDE SALTAR  derivado del codigo: sobre que kinds itera la funcion
                         que emite cada codigo
  B. LA VALIDACION       la derivacion es una aproximacion, asi que se coteja
                         contra casos cuya respuesta ya se conoce
  C. EL CRUCE            puede saltar en los dos x se prueba en uno = ciego
  D. EL GUARDIA          cuantas entradas habria que clasificar a mano, y si
                         eso se sostiene
"""
import collections
import pathlib
import re

RAIZ = pathlib.Path(r"C:\ORE")
CRATES = RAIZ / "crates"
CONF = RAIZ / "vendor/oos/conformance"

# Vocabulario de cada paradigma, tal como aparece en el codigo.
VIEJO = {"Kind::Binding"}
NUEVO = {"Kind::View", "Kind::Table", "pkg.entities()", "respaldo(", "backedBy"}


def funciones(texto):
    """Trocea un fichero en funciones. `fn nombre` hasta el siguiente `fn` de
    la misma indentacion, que basta: lo que se busca es el ambito lexico en el
    que vive una emision, no un arbol de sintaxis."""
    marcas = [(m.start(), m.group(1)) for m in re.finditer(r"^(?:    )*(?:pub )?fn (\w+)",
                                                           texto, re.M)]
    for i, (ini, nombre) in enumerate(marcas):
        fin = marcas[i + 1][0] if i + 1 < len(marcas) else len(texto)
        yield nombre, texto[ini:fin]


print("== el censo de paridad ==")

# -- A - DONDE PUEDE SALTAR --------------------------------------------------
print()
print("A - DONDE PUEDE SALTAR CADA CODIGO, derivado del codigo")
puede = collections.defaultdict(set)
donde_vive = collections.defaultdict(set)
for f in sorted(CRATES.rglob("*.rs")):
    if "tests" in f.parts:
        continue
    texto = f.read_text(encoding="utf-8", errors="replace")
    for nombre, cuerpo in funciones(texto):
        # Los comentarios no ejecutan nada: hablan de kinds sin tocarlos.
        codigo = "\n".join(l for l in cuerpo.split("\n")
                           if not l.strip().startswith("//"))
        emitidos = set(re.findall(r"Code::Oos(\d{4})", codigo))
        if not emitidos:
            continue
        v = any(k in codigo for k in VIEJO)
        n = any(k in codigo for k in NUEVO)
        for c in emitidos:
            puede["OOS" + c].add("viejo" if v else None)
            puede["OOS" + c].add("nuevo" if n else None)
            donde_vive["OOS" + c].add("%s::%s" % (f.name, nombre))
for c in puede:
    puede[c].discard(None)

reparto = collections.Counter()
for c, p in puede.items():
    reparto["los dos" if len(p) == 2 else (next(iter(p)) if p else "ninguno")] += 1
for k, v in reparto.most_common():
    print("   %-10s %3d codigos" % (k, v))
print()
print("   «ninguno» = la funcion que lo emite no menciona ningun kind de")
print("   sustrato: son reglas de forma, de tipos o de gobierno, y valen igual")
print("   en los dos paradigmas porque no hablan de ninguno.")

# -- B - LA VALIDACION -------------------------------------------------------
print()
print("B - LA VALIDACION: la derivacion es una aproximacion, y se coteja")
CONOCIDOS = [
    # La primera expectativa aqui decia «nuevo» y era MIA, no de la derivacion:
    # `OOS4011` lo emiten el sello del indice Y el eje de un binding, asi que
    # los dos es la respuesta correcta. Se deja escrito porque un cotejo que se
    # ajusta al resultado no coteja nada.
    ("OOS4011", "los dos", "lo emiten el sello del indice y el eje del binding"),
    ("OOS2014", "viejo", "«dos bindings del mismo objeto reclaman la misma fila»"),
    ("OOS2025", "nuevo", "una vista por la que se escribe"),
    ("OOS2027", "ninguno", "`exports` del manifiesto: no habla de sustrato"),
]
fallos = 0
for c, esperado, por_que in CONOCIDOS:
    real = "los dos" if len(puede[c]) == 2 else (next(iter(puede[c])) if puede[c] else "ninguno")
    ok = real == esperado
    fallos += not ok
    print("   %-9s espera %-9s da %-9s %s  %s"
          % (c, esperado, real, "ok" if ok else "NO", por_que))
print()
if fallos:
    print("   -> %d de %d cotejos fallan: la derivacion NO se puede usar sola."
          % (fallos, len(CONOCIDOS)))
else:
    print("   -> los cuatro casan. La derivacion sirve para PROPONER, y aun asi")
    print("      no para decidir: dice que kinds toca la funcion, no si la regla")
    print("      TIENE SENTIDO en el otro paradigma. Eso es juicio.")

# -- C - EL CRUCE ------------------------------------------------------------
print()
print("C - EL CRUCE: puede en los dos x se prueba en uno = ciego")


def paradigma_del_caso(dir_caso):
    v = n = False
    for d in ("input", "before", "after"):
        p = dir_caso / d
        if not p.is_dir():
            continue
        for f in p.rglob("*.yaml"):
            t = f.read_text(encoding="utf-8", errors="replace")
            v |= bool(re.search(r"^kind:\s*Binding", t, re.M))
            n |= bool(re.search(r"^kind:\s*(Table|View)", t, re.M))
    return {x for x, y in (("viejo", v), ("nuevo", n)) if y}


prueba = collections.defaultdict(set)
for c in sorted(CONF.rglob("case.yaml")):
    t = c.read_text(encoding="utf-8", errors="replace")
    m = re.search(r"^expects:\s*(.+)$", t, re.M)
    if not m:
        continue
    p = paradigma_del_caso(c.parent)
    for cod in re.findall(r"OOS\d{4}", m.group(1)):
        # `p` vacio significa «el caso no usa vocabulario de sustrato», que NO
        # es lo mismo que «no hay caso». La primera version las junto y conto
        # 63 codigos sin caso cuando 98 tienen uno: la mayoria del corpus prueba
        # forma, tipos y gobierno sin tocar el sustrato.
        prueba[cod] |= p or {"sin sustrato"}

ciegos, correctos, sin_caso = [], [], []
for c in sorted(puede):
    pu, pr = puede[c], prueba.get(c, set())
    reales = pr - {"sin sustrato"}
    if not pr:
        sin_caso.append(c)
    elif len(pu) == 2 and len(reales) < 2:
        ciegos.append((c, "+".join(sorted(pu)), "+".join(sorted(pr)) or "-"))
    else:
        correctos.append(c)
print("   %-46s %3d" % ("codigos con emision localizada", len(puede)))
print("   %-46s %3d" % ("  CIEGOS: valen en los dos, se prueban en uno", len(ciegos)))
print("   %-46s %3d" % ("  sin ningun caso", len(sin_caso)))
print("   %-46s %3d" % ("  el resto", len(correctos)))
print()
for c, pu, pr in ciegos:
    print("     %-9s puede: %-14s se prueba: %s   %s"
          % (c, pu, pr, sorted(donde_vive[c])[0]))
print()
print("   Y esto es lo que valida el censo como MECANISMO, mas que el numero.")
print("   `medida-el-corpus` conto DIEZ codigos «probados solo con binding», y")
print("   esa cifra mezclaba dos cosas: los que no pueden saltar en el paradigma")
print("   nuevo —y entonces el caso viejo es el correcto— y los que si.")
print()
print("   Preguntando ademas DONDE PUEDE SALTAR, de los diez queda %d. Y es"
      % len(ciegos))
print("   `OOS4001`, que es exactamente el que resulto ser un defecto de verdad")
print("   —el sello no subia por la cadena—. El censo habria apuntado ahi sin")
print("   que nadie tropezara con ello buscando otra cosa.")

# -- D - EL GUARDIA ----------------------------------------------------------
print()
print("D - ¿SE PUEDE ESCRIBIR COMO GUARDIA?")
print("   Lo que la derivacion NO decide: si una regla que hoy solo toca")
print("   bindings DEBERIA tener sujeto nuevo. `OOS2014` —dos bindings del")
print("   mismo objeto— no lo tiene y es correcto; otra podria no tenerlo por")
print("   descuido, y las dos se ven igual desde el codigo.")
print()
print("   Asi que el guardia no puede derivarse entero. La forma que la casa ya")
print("   usa para esto es una lista escrita A MANO —`IMPLEMENTADAS`— y esta")
print("   medido lo que cuesta: «una lista escrita a mano aqui envejece en")
print("   silencio la primera vez que llega un borrador nuevo».")
print()
print("   El reparto dice cuanto habria que escribir:")
print("     %3d codigos con emision localizada" % len(puede))
print("     %3d de ellos NO tocan sustrato -> no entran en el censo"
      % sum(1 for c in puede if not puede[c]))
print("     %3d tocan uno o los dos -> son los que habria que clasificar"
      % sum(1 for c in puede if puede[c]))
print()
print("   -> el censo es de esa ultima cifra, no de 99. Y la derivacion no")
print("      sobra: propone la clasificacion, y la lista a mano solo tiene que")
print("      CONTRADECIRLA donde el juicio diga otra cosa — que es mucho menos")
print("      que escribirla entera, y ademas falla si alguien anade un codigo")
print("      nuevo sin decir de que paradigma es.")
