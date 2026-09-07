# -*- coding: utf-8 -*-
"""Tres preguntas: si el paquete esta cerrado, y cual de las otras va primero.

  A. LA PRIORIDAD 3, PIEZA A PIEZA   si esta cerrada, que lo dice
  B. LO QUE QUEDO FUERA A PROPOSITO  y por que no la reabre
  C. EL CAMINO DE ESCRITURA          que hay, y donde esta el hueco de verdad
  D. LA MAQUINA DE AGRUPAR           construida por los dos extremos, sin vocabulario
  E. EL ORDEN
"""
import json
import pathlib
import re
import subprocess
import textwrap

RAIZ = pathlib.Path(r"C:\ORE")


def parrafo(t, sangria="     ", ancho=70):
    for l in textwrap.wrap(t, ancho):
        print("%s%s" % (sangria, l))


def texto(ruta):
    try:
        return (RAIZ / ruta).read_text(encoding="utf-8", errors="replace")
    except OSError:
        return ""


def hay(patron, ruta):
    return re.search(patron, texto(ruta)) is not None


print("== tres prioridades, medidas ==")

# -- A ------------------------------------------------------------------------
print()
print("A - LA PRIORIDAD 3, PIEZA A PIEZA")
print()
PIEZAS = [
    ("la regla", r"pub const DEL_PAQUETE", "crates/ore-core/src/pertenencia.rs"),
    ("...corre antes de link", r"pertenencia::check", "crates/ore-core/src/validate.rs"),
    ("...y con censo", r"cada_kind_dice_de_que_poblacion_es",
     "crates/ore-core/src/pertenencia.rs"),
    ("`package new`", r"pub fn nuevo\(", "crates/ore-cli/src/paquete.rs"),
    ("`package move`", r"pub fn mover\(", "crates/ore-cli/src/paquete.rs"),
    ("`package split`", r"pub fn dividir\(", "crates/ore-cli/src/paquete.rs"),
    ("`package merge`", r"pub fn fundir\(", "crates/ore-cli/src/paquete.rs"),
    ("...los tres sobre un plan", r"fn planificar\(", "crates/ore-cli/src/paquete.rs"),
    ("...todo o nada", r"struct Taller", "crates/ore-cli/src/paquete.rs"),
    ("`OOS2030` en el registro", r"Oos2030", "crates/ore-core/src/code.rs"),
    ("`OOS2031` en el registro", r"Oos2031", "crates/ore-core/src/code.rs"),
    ("...y quien lo emite", r"Code::Oos2031", "crates/ore-core/src/link.rs"),
    # El paquete se especifica en v1alpha1, no en v1alpha8: la primera version
    # miro en `spec/v1alpha8/` y dio «NO» a tres secciones que estan escritas y
    # publicadas. Y el arnes corregido destapo lo que el roto tapaba: DOS
    # secciones numeradas `3.5`, una de ellas citada por la tabla de errores.
    ("la pertenencia, en OOS", r"### 3\.5 · La pertenencia",
     "vendor/oos/spec/v1alpha1/01-package.md"),
    ("la lapida, en OOS", r"### 3\.6 · La lapida|### 3\.6 · La lápida",
     "vendor/oos/spec/v1alpha1/01-package.md"),
    ("los dos codigos, en OOS", r"OOS2031", "vendor/oos/spec/v1alpha1/99-errors.md"),
]
print("   %-30s %s" % ("pieza", "esta"))
print("   " + "-" * 44)
for q, pat, ruta in PIEZAS:
    print("   %-30s %s" % (q, "si" if hay(pat, ruta) else "NO"))

# Ningun numero de seccion se repite: es lo que la ruta equivocada tapaba.
sec = re.findall(r"^### (\d+\.\d+)", texto("vendor/oos/spec/v1alpha1/01-package.md"), re.M)
dobles = sorted({s for s in sec if sec.count(s) > 1})
print("   %-30s %s" % ("...sin numeros repetidos", "NO: %s" % dobles if dobles else "si"))
# El hueco de §3.3 es ANTERIOR y esta citado desde cinco sitios: `exports` fue
# §3.3 y `b00d545` lo renumero a §3.2. Cerrarlo romperia esas cinco citas.
print("   %-30s %s" % ("...y el hueco de 3.3, anterior", "se deja"))

pruebas = texto("crates/ore-cli/tests/paquete.rs")
print()
print("   pruebas de integracion del mando: %d" % len(re.findall(r"#\[test\]", pruebas)))

# El caso de conformance se busca por NOMBRE de directorio, que es lo unico
# estable: buscarlo por texto dentro del yaml daria «NO» sobre un caso que esta.
conf = RAIZ / "vendor/oos/conformance/v1alpha8/invalid"
casos = sorted(p.name for p in conf.glob("*outside*")) if conf.exists() else []
print("   conformance del documento fuera de su paquete: %s"
      % (casos[0] if casos else "NO"))

# -- B ------------------------------------------------------------------------
print()
print("B - LO QUE QUEDO FUERA, Y POR QUE NO LA REABRE")
print()
FUERA = [
    ("`exports`", "decidido",
     "los verbos no lo tocan, y no es un olvido: «esto lo expongo a proposito» "
     "es una frase de gobierno, y un mando que ensancha la superficie publica "
     "por su cuenta contradice la frase para la que la lista existe"),
    ("cuando se retira una lapida", "pregunta de producto",
     "no de codigo, y su respuesta ya tiene sitio: `sla.breakingChangePolicy`, "
     "el unico campo normativo del SLA. Nada de lo construido la necesita para "
     "funcionar hoy"),
    ("el corte mas barato", "se dice, no se busca",
     "sobre un paquete modelado no hay corte gratis. `split` dice el precio y "
     "para; minimizarlo por su cuenta seria elegir el limite del dominio, que "
     "no es suyo"),
    ("dos asuntos iguales en el log", "publicado",
     "`1ad4a10` y `b86b467` comparten titular con arboles distintos. Es "
     "cosmetico y ya es historia: reescribirlo pide un force-push"),
]
for q, e, por in FUERA:
    print("   · %-28s %s" % (q, e))
    parrafo(por, "       ")
    print()

# -- C ------------------------------------------------------------------------
print()
print("C - EL CAMINO DE ESCRITURA, MEDIDO")
print()
parrafo("Lo primero que hay que deshacer es una confusion de nombres: "
        "`Table.spec.changes` NO es el camino de escritura. Es la cara `D` — "
        "que cambios EMITE el origen, para que el mantenedor incremental no "
        "los adivine. Va hacia dentro, no hacia fuera.")
print()
print("   La escritura de verdad son cuatro capas, y estan en cuatro estados:")
print()

code = texto("crates/ore-core/src/code.rs")
emisores = {}
for c in sorted(set(re.findall(r"Oos70\d\d", code))):
    salida = subprocess.run(["git", "grep", "-l", "Code::%s" % c, "--", "crates/"],
                            cwd=str(RAIZ), capture_output=True, text=True).stdout
    emisores[c] = [x for x in salida.splitlines() if "code.rs" not in x]
vivos = [c for c, f in emisores.items() if f]
muertos = [c for c, f in emisores.items() if not f]

llama = subprocess.run(["git", "grep", "-n", "invertible(", "--", "crates/"],
                       cwd=str(RAIZ), capture_output=True, text=True).stdout.splitlines()
# Las llamadas de la propia prueba viven en el `mod tests` de `vistas.rs`; se
# descartan por fichero, no por numero de linea, que se mueve con cada edicion.
fuera = [l for l in llama if not l.startswith("crates/ore-core/src/vistas.rs")]

drivers = sorted(p.name for p in (RAIZ / "crates").iterdir()
                 if re.match(r"ore-(read|write)-", p.name))
escriben = [d for d in drivers if d.startswith("ore-write-")]

print("   %-14s %-42s %s" % ("capa", "que es", "estado"))
print("   " + "-" * 74)
print("   %-14s %-42s %s"
      % ("VOCABULARIO", "`Function.effects` y sus endosos", "completo"))
print("   %-14s %-42s %s"
      % ("GOBIERNO", "OOS7001-7011", "%d de %d con emisor" % (len(vivos), len(emisores))))
print("   %-14s %-42s %s"
      % ("GUARDA", "`vistas::invertible`, escrita y probada",
         "%d llamadas fuera de su fichero" % len(fuera)))
print("   %-14s %-42s %s"
      % ("EJECUTOR", "un driver que escriba",
         "%d de %d drivers" % (len(escriben), len(drivers))))
print()
print("   sin emisor: %s" % (", ".join(muertos) if muertos else "ninguno"))
print()
parrafo("Y `OOS7013` sin emisor NO es un hueco: esta RESERVADO por decision "
        "—ADR 0018—. Escribir desde la ontologia aterriza en la COPIA, y una "
        "edicion cae DENTRO de `Q`, asi que el codigo existe para que nadie lo "
        "reutilice, no para dispararse.")
print()
parrafo("Asi que el camino de escritura no esta a medias: esta DECIDIDO que no "
        "se anda todavia, y el gobierno que lo precede si esta entero. Abrirlo "
        "no es completar algo empezado — es empezar un producto, y el primero "
        "de sus problemas ya tiene respuesta escrita en contra.")

# -- D ------------------------------------------------------------------------
print()
print("D - LA MAQUINA DE AGRUPAR")
print()
todas = sorted((RAIZ / "crates/ore-view/src").glob("*.rs"))
piezas = {}
for p in todas:
    n = len(re.findall(r"Agrupa|Agregado|Agregacion",
                       p.read_text(encoding="utf-8", errors="replace")))
    if n:
        piezas[p.stem] = n
print("   piezas de `ore-view` que la tocan: %d de %d  ·  %d menciones"
      % (len(piezas), len(todas), sum(piezas.values())))
for k, v in sorted(piezas.items(), key=lambda x: -x[1]):
    print("     %-20s %3d" % (k, v))
print()
GOB = [
    ("el desclasificador", r'"aggregate"', "crates/ore-core/src/flow.rs"),
    ("su umbral", r"minGroupSize", "crates/ore-core/src/cedar.rs"),
    ("`OOS4007`", r"Oos4007", "crates/ore-core/src/code.rs"),
    ("`OOS5016`", r"Oos5016", "crates/ore-core/src/code.rs"),
    ("la accion de Cedar", r'"aggregate"', "crates/ore-core/src/cedar_schema.rs"),
    ("`aggregatePushdown`", r"aggregatePushdown", "crates/ore-core/src/normalize.rs"),
]
print("   Y el GOBIERNO de agregar, construido antes que nada de esto:")
for q, pat, ruta in GOB:
    print("     %-22s %s" % (q, "si" if hay(pat, ruta) else "NO"))
print()
v = json.loads(texto("vendor/oos/schemas/v1alpha8/view.schema.json"))
claves = sorted(v["properties"]["spec"]["properties"])
print("   El vocabulario de `View`, entero:")
print("     %s" % ", ".join(claves))
print("     tiene `groupBy`: %s" % ("si" if "groupBy" in claves else "NO"))
print()
parrafo("Los dos extremos construidos y el medio vacio. El IR sabe agrupar, "
        "sabe MANTENER un agrupado incremental —la media se parte en suma y "
        "cuenta— y sabe ENROLLAR un agregado a una agrupacion mas gruesa. El "
        "gobierno sabe que agregar DESCLASIFICA y exige un tamano minimo de "
        "grupo. Y ningun documento puede pedir una agrupacion: `groupBy` no "
        "pasa de `OOS1005`.")
print()
parrafo("Esto no lo descubre esta medida: `vistas.rs` lo confiesa en su propia "
        "prueba —«el IR de `ore-view` tiene `Agrupa` probado sin que ningun "
        "documento lo produzca»—. Lo que la medida anade es EL TAMANO: no es un "
        "nodo suelto, son nueve piezas y el regimen de flujo entero.")

# -- E ------------------------------------------------------------------------
print()
print("E - EL ORDEN")
print()
ORDEN = [
    ("`groupBy` en `View`", "enciende lo construido",
     "la unica de las cuatro que no construye maquina: la maquina esta, y "
     "gobernada. El coste esta en el vocabulario, en `invertible` —que ya sabe "
     "decir que no— y en el compilador de vista a plan"),
    ("el rango por posicion", "cierra una deuda",
     "hay un sustituidor por posicion escrito para los verbos de paquete y "
     "nadie mas lo ejerce. Es pequeno y no abre superficie"),
    ("`ore-read-mysql`", "ensancha lo que hay",
     "el cuarto driver sobre una forma de catalogo que ya prueban tres. Es "
     "trabajo conocido: su valor es medir si la forma aguanta, no descubrir"),
    ("el camino de escritura", "es otro producto",
     "no esta a medias, esta decidido que no. Abrirlo obliga a reabrir donde "
     "aterriza la escritura, y ADR 0018 ya contesto que en la copia"),
]
for q, e, por in ORDEN:
    print("   · %-24s %s" % (q, e))
    parrafo(por, "       ")
    print()
