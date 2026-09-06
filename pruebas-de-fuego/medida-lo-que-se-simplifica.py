# -*- coding: utf-8 -*-
"""Lo que se simplifica al quedar UN SOLO paradigma — y lo que no.

`medida-retirar-binding` §C nombro tres cosas que existen «solo porque habia
dos paradigmas», y las dio por simplificables las tres:

    1. `Arista::derivada`      — el campo que separaba los dos caminos
    2. EL CENSO DE PARIDAD     — «que regla vale en los dos y se prueba en uno»
    3. LAS ACOTACIONES v1alpha8 — «sin resultados viejos que preservar, es ruido»

Esto las coteja UNA A UNA contra el arbol, porque la tercera se escribio de
oido: «resultados viejos» no queria decir «repositorios de clientes» —eso es lo
que el claim del usuario desecho— sino LA MITAD SIN MIGRAR DE NUESTRO PROPIO
CORPUS, que sigue ahi y sigue en verde.

Y un cuarto frente, que no estaba en la lista y es el que mas se mueve:

    4. EL PELDANO 6, cuya metrica decisiva —«N entidades llegan por binding»—
       dejo de ser cierta el dia que el motor dejo de leer bindings.
"""
import collections
import pathlib
import re
import subprocess

RAIZ = pathlib.Path(r"C:\ORE")
CRATES = RAIZ / "crates"
OOS = RAIZ / "vendor/oos"
CONF = OOS / "conformance"

VIEJO = {"Kind::Binding"}
NUEVO = {"Kind::View", "Kind::Table", "pkg.entities()", "respaldo(", "backedBy"}


def documentos():
    """Cada documento YAML del corpus: kind, texto, fichero."""
    for r in (OOS, RAIZ / "casos"):
        if not r.is_dir():
            continue
        for f in sorted(r.rglob("*.yaml")):
            t = f.read_text(encoding="utf-8", errors="replace")
            for d in re.split(r"^---\s*$", t, flags=re.M):
                k = re.search(r"^kind:\s*(\w+)", d, re.M)
                if k:
                    yield k.group(1), d, f


def version(d):
    m = re.search(r"^apiVersion:\s*oos\.dev/v1alpha(\d+)", d, re.M)
    return int(m.group(1)) if m else None


def funciones(texto):
    marcas = [(m.start(), m.group(1))
              for m in re.finditer(r"^(?:    )*(?:pub )?fn (\w+)", texto, re.M)]
    for i, (ini, nombre) in enumerate(marcas):
        fin = marcas[i + 1][0] if i + 1 < len(marcas) else len(texto)
        yield nombre, texto[ini:fin]


TODOS = list(documentos())
print("== lo que se simplifica, y lo que no ==")

# -- A - `Arista::derivada` --------------------------------------------------
print()
print("A - `Arista::derivada`: el campo que ya no distingue nada")
ar = (CRATES / "ore-core/src/aristas.rs").read_text(encoding="utf-8")
flow = (CRATES / "ore-core/src/flow.rs").read_text(encoding="utf-8")
asignaciones = re.findall(r"^\s+derivada:\s*(\w+),", ar, re.M)
filtros = len(re.findall(r"filter\(\|a\| a\.derivada\)", flow))
print("   valores que el constructor le da : %s" % ", ".join(sorted(set(asignaciones))))
print("   sitios que filtran por el         : %d" % filtros)
print("   lineas de comentario que lo justifican:")
just_ar = len(re.findall(r"\n", re.search(
    r"    /// Si la fuente la declara.*?\n    pub derivada", ar, re.S).group(0)))
m = re.search(r"/// # Por qu[eé] solo las derivadas.*?(?=/// # )", flow, re.S)
just_flow = len(m.group(0).rstrip().split("\n")) if m else 0
# Y en esa justificacion vive ademas un enlace a `materializaciones`, que se
# borro con el eje del binding: el comentario no solo sobra, ya apunta a nada.
print("     ...y cita `[materializaciones]`, ya borrada : %s"
      % ("si" if m and "materializaciones" in m.group(0) else "no"))
print("     `aristas.rs` (el campo)        : %2d" % just_ar)
print("     `flow.rs` (por que se filtra)  : %2d" % just_flow)
docs_men = subprocess.run(["grep", "-rln", r"Arista::derivada", str(RAIZ / "docs")],
                          capture_output=True, text=True).stdout.split()
print("     `docs/` que lo nombran         : %s"
      % (", ".join(pathlib.Path(x).name for x in docs_men) or "(ninguno)"))
print()
print("   El campo decia «esta fuente la declara una VISTA, no un binding», y")
print("   servia para no sellar dos veces el mismo camino. Con un solo camino")
print("   el constructor solo puede escribir `true`, el filtro no descarta")
print("   nada, y la justificacion habla de una rama que no existe.")
print()
print("   -> se cae SOLO: campo, filtro y %d lineas de comentario. Cero riesgo,"
      % (just_ar + just_flow))
print("      porque no hay ningun valor `false` que dejara de descartarse.")

# -- B - LAS ACOTACIONES POR VERSION -----------------------------------------
print()
print("B - LAS ACOTACIONES v1alpha8: el sujeto que SI existe")
por_ver = collections.Counter()
for k, d, f in TODOS:
    por_ver[version(d)] += 1
viejos = sum(v for k, v in por_ver.items() if k is not None and k < 8)
nuevos = sum(v for k, v in por_ver.items() if k is not None and k >= 8)
print("   documentos del corpus por version:")
for k in sorted(x for x in por_ver if x is not None):
    print("     v1alpha%-2d %5d" % (k, por_ver[k]))
print("   %-14s %5d  <- lo que cada acotacion protege" % ("< v1alpha8", viejos))
print("   %-14s %5d" % (">= v1alpha8", nuevos))
print()


def sujeto(pred):
    a = b = 0
    for k, d, f in TODOS:
        if not pred(k, d):
            continue
        v = version(d)
        if v is not None and v < 8:
            a += 1
        else:
            b += 1
    return a, b


ACOTACIONES = [
    ("OOS1005", "effect.rs:349", "`Function` con `datasourceRef` en un efecto",
     lambda k, d: k == "Function" and "datasourceRef" in d),
    ("OOS2009", "link.rs:382", "`Lattice` / `OntologyConfig`",
     lambda k, d: k in ("Lattice", "OntologyConfig")),
    ("OOS2022", "vistas.rs:1221", "`Entity` con `backedBy`",
     lambda k, d: k == "Entity" and re.search(r"^\s+backedBy:", d, re.M)),
    ("OOS2028", "exporta.rs:282", "cualquier documento que cruce de paquete",
     lambda k, d: True),
]
print("   %-9s %-16s %-42s %6s %6s" % ("codigo", "donde", "sujeto", "<a8", ">=a8"))
print("   " + "-" * 86)
for cod, donde, que, pred in ACOTACIONES:
    a, b = sujeto(pred)
    print("   %-9s %-16s %-42s %6d %6d" % (cod, donde, que, a, b))
print()
print("   Ninguna esta sin sujeto, y dos tienen MAS sujeto viejo que nuevo. El")
print("   argumento de §C —«sin resultados viejos que preservar»— confundio dos")
print("   cosas: el claim del usuario retiro LA AUDITORIA DE TERCEROS, no las")
print("   versiones. Lo que estas puertas protegen no es un cliente: es")
print("   %d documentos NUESTROS que nadie ha migrado." % viejos)
print()
print("   Y esta escrito lo que cuesta quitarlas, medido en su dia:")
print("     `OOS2022` sin puerta -> `conformance/v1alpha7` 13/13 a 12/13, y")
print("                             `acme-retail` deja de validar")
print("     `OOS2028` sin puerta -> caen los DOS unicos cruces del corpus")
print()
print("   -> NO se simplifica quitandolas. Se simplifica MIGRANDO EL CORPUS, y")
print("      entonces se caen solas por falta de sujeto. Es el orden inverso")
print("      al que §C proponia, y es el unico que no rompe nada.")

# -- C - EL CENSO DE PARIDAD -------------------------------------------------
print()
print("C - EL CENSO DE PARIDAD: sin objeto")
puede = collections.defaultdict(set)
for f in sorted(CRATES.rglob("*.rs")):
    if "tests" in f.parts:
        continue
    for nombre, cuerpo in funciones(f.read_text(encoding="utf-8", errors="replace")):
        codigo = "\n".join(l for l in cuerpo.split("\n")
                           if not l.strip().startswith("//"))
        emitidos = set(re.findall(r"Code::Oos(\d{4})", codigo))
        if not emitidos:
            continue
        v = any(x in codigo for x in VIEJO)
        n = any(x in codigo for x in NUEVO)
        for c in emitidos:
            puede["OOS" + c].add("viejo" if v else None)
            puede["OOS" + c].add("nuevo" if n else None)
for c in puede:
    puede[c].discard(None)
reparto = collections.Counter(
    "los dos" if len(p) == 2 else (next(iter(p)) if p else "ninguno")
    for p in puede.values())
for k in ("los dos", "viejo", "nuevo", "ninguno"):
    print("   %-10s %3d codigos" % (k, reparto[k]))
print()
print("   La pregunta del censo era «que regla vale en los DOS y solo se prueba")
print("   en uno». Con %d codigos en «los dos» y %d en «viejo», la interseccion"
      % (reparto["los dos"], reparto["viejo"]))
print("   esta vacia por construccion: no hay dos paradigmas que cruzar.")
print()
print("   Lo que NO se cae con el: el censo encontro `OOS4001` —el sello que no")
print("   subia por la cadena— y ese defecto era real. Lo que valia no era el")
print("   eje «viejo/nuevo»: era preguntar DONDE PUEDE SALTAR frente a DONDE SE")
print("   PRUEBA. Ese cruce sigue teniendo sujeto con un solo paradigma, y es")
print("   la parte que merece sobrevivir al censo.")

# -- D - EL PELDANO 6, cuya metrica se movio ---------------------------------
print()
print("D - EL PELDANO 6: la metrica que dejo de ser cierta")
ents = [(d, f) for k, d, f in TODOS if k == "Entity"]
con_bb = [1 for d, f in ents if re.search(r"^\s+backedBy:", d, re.M)]
sin_bb = [(d, f) for d, f in ents if not re.search(r"^\s+backedBy:", d, re.M)]
bindings = subprocess.run(["grep", "-rl", "^kind: Binding", str(OOS), str(RAIZ / "casos"),
                           "--include=*.yaml"], capture_output=True, text=True).stdout.split()
print("   `medida-espectro-fusion` §E decia, y era su tramo 4:")
print("     «%d entidades llegan por `Binding`, que no caduca. O sea que la"
      % len(sin_bb))
print("      fusion NO puede retirar `Entity`: tendria que convivir con ella")
print("      para siempre.»")
print()
print("   Eso se derivo de «no tienen `backedBy`», y se LEYO como «llegan por")
print("   binding». Contando los bindings de verdad:")
print("     %-46s %4d" % ("entidades del corpus", len(ents)))
print("     %-46s %4d" % ("  ...con `backedBy`", len(con_bb)))
print("     %-46s %4d" % ("  ...sin `backedBy`", len(sin_bb)))
print("     %-46s %4d" % ("ficheros `kind: Binding` en TODO el corpus", len(bindings)))
for b in bindings:
    print("       %s" % pathlib.Path(b).relative_to(RAIZ).as_posix())
sin_nada = 0
for d, f in sin_bb:
    pk = f.parent.parent
    if not any(re.search(r"^kind:\s*Binding",
                         p.read_text(encoding="utf-8", errors="replace"), re.M)
               for p in pk.rglob("*.yaml")):
        sin_nada += 1
print("     %-46s %4d" % ("entidades sin `backedBy` NI binding en su paquete", sin_nada))
print()
print("   -> los tres bindings que quedan viven en `docs/vision/`, que")
print("      `examples.rs` excluye a proposito porque NO VALIDA. Asi que el")
print("      camino viejo no es un camino: %d de %d entidades no llegan por"
      % (sin_nada, len(sin_bb)))
print("      binding — no llegan por NADA. Son significado sin sustrato.")

# El buque insignia, que es donde esto deja de ser una cifra.
print()
print("   Y donde se ve mejor es en lo unico que OOS publica y valida:")
insignia = OOS / "examples/acme-retail"
filas = []
for f in sorted(insignia.rglob("*.yaml")):
    t = f.read_text(encoding="utf-8", errors="replace")
    if re.search(r"^kind:\s*Entity", t, re.M):
        bb = re.search(r"^\s+backedBy:\s*(\S+)", t, re.M)
        filas.append((f.stem, bb.group(1) if bb else None))
for n, bb in filas:
    print("     %-12s %s" % (n, ("-> %s" % bb) if bb else "(sin sustrato)"))
salida = subprocess.run([str(RAIZ / "target/debug/ore"), "validate", str(insignia)],
                        capture_output=True, text=True).stdout
veredicto = [l.strip() for l in salida.split("\n") if re.match(r"\s*(ok|error)", l)]
print("   `ore validate examples/acme-retail` : %s"
      % (veredicto[0] if veredicto else "(?)"))
print()
print("   %d de %d entidades del buque insignia no tienen de donde salir, y"
      % (sum(1 for _, bb in filas if not bb), len(filas)))
print("   valida en verde. Ninguna regla lo mira: `sin_respaldo` existe y solo")
print("   la leen los EMISORES —GraphQL y Ossie—, que fallan tarde y solo si se")
print("   les llama. Es el mismo modo de fallo de la casa: lo que falta se")
print("   parece demasiado a lo que esta bien.")
print()
print("   LO QUE ESTO LE HACE AL PELDANO 6")
print("     antes: «`Entity` no se retira porque el camino del binding no")
print("            caduca» — una convivencia permanente, tramo 4 del espectro.")
print("     ahora: ese camino no existe. Lo que sostiene a `Entity` no es una")
print("            version vieja: son %d entidades que declaran significado" % sin_nada)
print("            sin sustrato ninguno, y la fusion no les da sitio — porque")
print("            no hay vista a la que mover el significado.")
print()
print("     Y con eso el tramo 4 cambia de naturaleza: deja de ser «sostener")
print("     dos caminos para siempre» y pasa a ser una PREGUNTA CONTESTABLE —")
print("     una entidad sin vista, ¿es un error que nadie emite, o es una")
print("     figura legitima —significado sin datos, como un vocabulario—?")
print("     Segun se conteste, el peldano 6 retira `Entity` o no la retira. Y")
print("     hoy esa pregunta se puede contestar; hace una semana no.")
