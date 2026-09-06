# -*- coding: utf-8 -*-
"""L2: el terreno. Que es un nivel de conformidad y por que este no se puede
reclamar.

`05-ejecutor.md` acaba de pasar a historico, y era el documento que DEFINE L2.
Eso no rompio nada nuevo: solo dejo visible que el nivel llevaba definido sobre
un vocabulario retirado desde que se suprimio `ore-exec`. Seis frentes:

  A. QUE ES UN NIVEL   los tres, y en que se distinguen de verdad
  B. QUIEN LOS USA     cada caso de conformidad declara el suyo. Cuantos hay
                       de cada uno, que es lo que dice si el nivel existe
  C. LA DEFINICION     donde se enuncia L2, y con que palabras
     ROTA
  D. QUE SOBREVIVE     las reglas de L2 no murieron: cambiaron de sujeto.
                       Cuantas, y donde estan hoy
  E. QUIEN PODRIA      que tendria que poder hacer una implementacion para
     RECLAMARLO        reclamarlo, y cuanto de eso existe
  F. LA FORMA DE LA    lo que hay que decidir, y las opciones que el arbol
     DECISION          admite
"""
import collections
import pathlib
import re
import subprocess

RAIZ = pathlib.Path(r"C:\ORE")
OOS = RAIZ / "vendor/oos"
SPEC = OOS / "spec"
CONF = OOS / "conformance"


def grep(patron, donde, ext="*.md"):
    return subprocess.run(["grep", "-rn", patron, str(donde), "--include=" + ext],
                          capture_output=True, text=True).stdout.strip().split("\n")


print("== L2, el terreno ==")

# -- A - QUE ES UN NIVEL -----------------------------------------------------
print()
print("A - LOS TRES NIVELES, y en que se distinguen de verdad")
print("   Lo dice `05-ejecutor` §1, y la distincion no es de tamano:")
print()
print("     L0   hermetico            compila y no toca un dato")
print("     L1   sirve el ARTEFACTO   el plano de CONTEXTO: entidades,")
print("                               relaciones, tipos, politicas, linaje")
print("     L2   sirve el DATO        el plano de DATOS: filas y valores")
print()
print("   Y la frase que los parte, que es la unica que hay que retener:")
print("     «L0 y L1 fallan AL COMPILAR. L2 falla AL RESPONDER.»")
print()
print("   Por eso L2 casi no anade codigos: un rechazo en tiempo de consulta no")
print("   es un defecto de un documento, es una CONDICION NOMBRADA que la")
print("   implementacion debe comunicar. Los `OOSxxxx` son del artefacto.")

# -- B - QUIEN LOS USA -------------------------------------------------------
print()
print("B - QUIEN LOS USA: cada caso de conformidad declara su nivel")
niveles = collections.Counter()
por_version = collections.defaultdict(collections.Counter)
for c in sorted(CONF.rglob("case.yaml")):
    t = c.read_text(encoding="utf-8", errors="replace")
    m = re.search(r"^level:\s*(\S+)", t, re.M)
    n = m.group(1) if m else "(sin nivel)"
    niveles[n] += 1
    rel = c.relative_to(CONF).parts[0]
    por_version[rel if rel.startswith("v1alpha") else "v1alpha1"][n] += 1
total = sum(niveles.values())
for n, v in niveles.most_common():
    print("   %-12s %3d casos   %s" % (n, v, "#" * (40 * v // max(total, 1))))
print()
print("   -> %d casos, y **%d de ellos son L2**. La suite entera comprueba el"
      % (total, niveles.get("L2", 0)))
print("      artefacto; el plano de datos no lo ejerce nadie, y no por descuido:")
print("      un caso de conformidad es un ARBOL DE FICHEROS y una respuesta")
print("      esperada. Un nivel que falla al responder no cabe en esa forma.")

# -- C - LA DEFINICION ROTA --------------------------------------------------
print()
print("C - DONDE SE ENUNCIA L2, Y CON QUE PALABRAS")
def partir(linea):
    """`C:\\ORE\\...\\x.md:93:texto` — la unidad de disco tambien lleva `:`.

    Partir por el primer `:` daba «fichero C» y «1 ficheros», que es un
    resultado inventado con la forma de uno medido.
    """
    m = re.match(r"^([A-Za-z]:[^:]*|[^:]+):(\d+):(.*)$", linea)
    return (pathlib.Path(m.group(1)).name, m.group(3).strip()) if m else (linea, "")


usos = [u for u in grep(r"\bL2\b", SPEC) if u]
print("   menciones en la spec: %d, en %d ficheros"
      % (len(usos), len({partir(u)[0] for u in usos})))
print()
for u in usos:
    f, txt = partir(u)
    if "resuelve" in txt or "Ejecutor" in txt or "nivel" in txt.lower():
        print("     %-22s %s" % (f, txt[:74]))
print()
print("   La definicion operativa es esa fila de `00-overview`:")
print("     «L2 · Ejecutor · resuelve BINDINGS contra fuentes reales»")
print()
print("   -> nombra `Binding`, que es historico, y `05-ejecutor`, que acaba de")
print("      pasar a historico tambien. El nivel no esta mal escrito: esta")
print("      definido sobre un vocabulario que ya no se escribe. Una")
print("      implementacion nueva —que solo tiene tablas y vistas— no puede")
print("      reclamarlo, porque no sabe contra que se mide.")

# -- D - QUE SOBREVIVE -------------------------------------------------------
print()
print("D - LO QUE NO MURIO: las reglas de L2 cambiaron de sujeto")
MUDANZAS = [
    ("§2 · el ejecutor no compensa", "ore-view::view_matcher",
     RAIZ / "crates/ore-view/src/view_matcher.rs", "compensation"),
    ("§5 · `fullScan` es autorizacion", "`OOS2020` + `01-table` §4",
     RAIZ / "crates/ore-core/src/code.rs", "Oos2020"),
    ("§6 · credenciales", "`connectionEnv` / `refreshEnv`",
     RAIZ / "crates/ore-cli/src/fuente.rs", "connectionEnv"),
    ("§7 · la marca de agua", "`changes.witness` + `registro::marca_de`",
     RAIZ / "crates/ore-cli/src/registro.rs", "fn marca_de"),
]
for seccion, donde, f, pat in MUDANZAS:
    vivo = pat in f.read_text(encoding="utf-8", errors="replace") if f.exists() else False
    print("   %-34s -> %-36s %s" % (seccion, donde, "vivo" if vivo else "NO ESTA"))
print()
print("   -> cuatro de las nueve secciones tienen sujeto nuevo y comprobable.")
print("      Lo que NO se mudo es la definicion del NIVEL, que es lo unico que")
print("      este documento tenia de propio.")

# -- E - QUIEN PODRIA RECLAMARLO ---------------------------------------------
print()
print("E - QUE HARIA FALTA PARA RECLAMAR L2, y cuanto existe")
PIEZAS = [
    ("un plan que se pueda ejecutar", RAIZ / "crates/ore-view", None),
    ("un protocolo de driver", RAIZ / "crates/ore-driver", None),
    ("al menos un driver", RAIZ / "crates/ore-read-postgres", None),
    ("un almacen para la copia", RAIZ / "crates/ore-store-r2", None),
    ("el ciclo de materializacion", RAIZ / "crates/ore-cli/src/materializar.rs", None),
    ("mantenimiento incremental", RAIZ / "crates/ore-maintain", None),
    ("servir el plano de contexto (L1)", RAIZ / "crates/ore-cli/src/mcp.rs", None),
]
for nombre, ruta, _ in PIEZAS:
    print("   %-38s %s" % (nombre, "existe" if ruta.exists() else "NO"))
drivers = sorted(p.name.replace("ore-read-", "")
                 for p in (RAIZ / "crates").iterdir() if p.name.startswith("ore-read-"))
tipos = set()
for f in OOS.rglob("ontology.config.yaml"):
    tipos |= set(re.findall(r"type:\s*(\w+)", f.read_text(encoding="utf-8", errors="replace")))
print()
print("   drivers que existen : %s" % ", ".join(drivers))
print("   tipos declarados    : %s" % ", ".join(sorted(tipos)))
print("   sin driver          : %s" % ", ".join(sorted(tipos - set(drivers))))
print()
print("   -> la maquinaria de L2 esta CASI ENTERA. Lo que falta no es el nivel:")
print("      son fuentes que sepa abrir. Y eso convierte la deuda de L2 en una")
print("      pregunta util en vez de una etiqueta rota.")

# -- F - LA FORMA DE LA DECISION ---------------------------------------------
print()
print("F - LA FORMA DE LA DECISION")
print("   No es «arreglar una frase». Son tres preguntas, y solo la primera es")
print("   barata:")
print()
print("   1. SOBRE QUE se define L2 ahora. La traduccion directa —cambiar")
print("      «resuelve bindings» por «resuelve VISTAS contra fuentes reales»—")
print("      es correcta y de una linea, y `01-table`/`02-view` ya dan el")
print("      vocabulario: la cara `reads`, la raiz de lectura, `materialized`.")
print()
print("   2. COMO SE COMPRUEBA. Y aqui esta lo de verdad: %d de %d casos son L2,"
      % (niveles.get("L2", 0), total))
print("      porque un caso es un arbol de ficheros y una respuesta esperada, y")
print("      L2 falla AL RESPONDER. O el nivel se reclama sin suite —y entonces")
print("      es una promesa, no una conformidad— o hace falta otra FORMA de")
print("      caso: un origen reproducible y una consulta. `jsonl` lo hace")
print("      posible sin servidor, y esa es la puerta mas barata que hay.")
print()
print("   3. SI L2 SIGUE SIENDO UN SOLO NIVEL. Hoy junta tres cosas que ya no")
print("      viajan juntas: LEER de una fuente, MATERIALIZAR una copia y")
print("      MANTENERLA. Las tres tienen crate propio y estado distinto, y una")
print("      implementacion podria hacer la primera y no las otras dos.")
