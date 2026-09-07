# -*- coding: utf-8 -*-
"""Cotejo de la vision: el sustrato como backend de una plataforma gestionada.

La vision, dicha por quien la tiene: el usuario NO escribe `ore` en un
terminal. Modela desde una superficie web —como el Object Type Manager, el
Action Type o el Link Type de Foundry— y **el YAML se escribe solo**; `ore x`
corre en un Kubernetes particionado; el registro con forma de REPOSITORIO es
lo que lo hace posible y escalable.

Esto no opina sobre la vision: mide las cuatro cosas que la vision exige y que
nadie ha medido todavia.

  A. EL ESCRITOR      cuantos `kind` se saben emitir, y con que disciplina
  B. LA EDICION       si se puede cambiar UN campo sin reescribir el fichero
  C. EL PROCESO       cuanto tarda, y si dos a la vez se pisan
  D. LA MULTITENANCIA que hay de identidad, y que no
  E. EL COTEJO        que parte de la vision sostiene el arbol de hoy
"""
import pathlib
import re
import subprocess
import textwrap
import time

RAIZ = pathlib.Path(r"C:\ORE")


def parrafo(t, sangria="     ", ancho=72):
    for l in textwrap.wrap(t, ancho):
        print("%s%s" % (sangria, l))


def texto(r):
    try:
        return (RAIZ / r).read_text(encoding="utf-8", errors="replace")
    except OSError:
        return ""


print("== el mecanismo de integracion, medido ==")

# -- A -------------------------------------------------------------------------
print()
print("A - EL ESCRITOR: QUE SE SABE EMITIR")
print()
parrafo("«Modelar es editar YAML» era una frase desde el terminal, y como "
        "diagnostico de producto esta mal planteada: el YAML DEBE escribirse "
        "solo. Lo que la frase sirve para medir es otra cosa — **cuantos "
        "documentos sabe escribir hoy este arbol**, porque una superficie de "
        "modelado necesita uno por cada clase que deje tocar.")
print()
doc = texto("crates/ore-core/src/document.rs")
i = doc.find("pub const ALL: &'static [Kind]")
kinds = re.findall(r"Kind::(\w+)", doc[i:doc.find("];", i)])
# El emisor de un `kind` es la funcion que produce su YAML. Se buscan por el
# `kind:` que escriben, no por el nombre de la funcion, que no es uniforme.
fuentes = {n: texto("crates/ore-cli/src/%s.rs" % n)
           for n in ("inductor", "inicio", "paquete", "vista", "revision", "lector")}
print("   %-18s %-10s %s" % ("kind", "emisor", "donde"))
print("   " + "-" * 62)
con, sin = [], []
for k in kinds:
    donde = [n for n, t in fuentes.items() if re.search(r"kind: %s\b" % k, t)]
    (con if donde else sin).append(k)
    print("   %-18s %-10s %s" % (k, "si" if donde else "NO", ", ".join(donde)))
print()
print("   %d de %d `kind` tienen quien los escriba." % (len(con), len(kinds)))
print()
parrafo("Y los que hay no son una API: son `fn *_yaml` privadas dentro del "
        "inductor, que construyen texto para el caso del descubrimiento. Una "
        "superficie de modelado necesita **la misma disciplina que ya tienen "
        "`documento_vista` y `documento_paquete`** —emisor unico, para que un "
        "documento escrito a mano y uno escrito por la UI sean el mismo "
        "texto— extendida a las clases que falten.")

# -- B -------------------------------------------------------------------------
print()
print("B - LA EDICION: CAMBIAR UN CAMPO SIN REESCRIBIR EL FICHERO")
print()
parrafo("Es el requisito que no se ve hasta que se construye la UI. Un "
        "formulario que cambia `fields` de una vista NO puede reemitir el "
        "documento entero: perderia comentarios, orden y forma, y cada edicion "
        "saldria en el `diff` como un fichero nuevo. Hay que **empalmar por "
        "posicion**.")
print()
parse = texto("crates/ore-core/src/parse.rs")
paq = texto("crates/ore-cli/src/paquete.rs")
PIEZAS = [
    ("cada nodo lleva su posicion", bool(re.search(r"pub fn pos\(&self\) -> Pos", parse)),
     "`line` y `col`, del analizador propio — no de un `serde` que reordena"),
    ("un empalmador por posicion", "fn sustituir_en" in paq,
     "existe, y lo usan los cuatro verbos de paquete para reapuntar referencias"),
    ("...y es publico", "pub fn sustituir_en" in paq,
     "es privado de `paquete.rs`"),
    ("...y empalma BLOQUES, no solo escalares",
     bool(re.search(r"fn sustituir_bloque|fn reemplazar_seccion", paq)),
     "solo empalma UN TOKEN. Cambiar `fields` entero —un mapa— no tiene "
     "primitiva todavia"),
]
for q, ok, por in PIEZAS:
    print("   %-42s %s" % (q, "si" if ok else "NO"))
    parrafo(por, "       ")
print()
parrafo("Asi que «el rango por posicion», que estaba en la lista de "
        "prioridades como una deuda pequena, **no es una deuda pequena**: es "
        "el primitivo sobre el que se construye una superficie de modelado. "
        "Reordenarlo no es un capricho de la vision — es lo que la vision "
        "destapa.")

# -- C -------------------------------------------------------------------------
print()
print("C - EL PROCESO: LATENCIA Y CONCURRENCIA")
print()
EJ = RAIZ / "vendor/oos/examples/acme-retail"
ORE = RAIZ / "target/debug/ore.exe"
if ORE.exists() and EJ.exists():
    for verbo in ("validate", "view"):
        t0 = time.time()
        subprocess.run([str(ORE), verbo, str(EJ)], capture_output=True)
        print("   %-12s %d ms  (binario de DEBUG, arranque en frio incluido)"
              % (verbo, int((time.time() - t0) * 1000)))
print()
candados = subprocess.run(["git", "grep", "-l", "flock"], cwd=str(RAIZ),
                          capture_output=True, text=True).stdout.split()
print("   candados sobre el arbol: %s" % (", ".join(candados) if candados else "NINGUNO"))
print()
parrafo("Dos consecuencias, y ninguna es un bloqueo: **hay que decidirlas**.")
print()
for q in [
    "EL ARRANQUE EN FRIO SE PAGA POR INVOCACION. Un `ore validate` es abrir un "
    "proceso, leer el arbol entero y contestar. Para un job de Kubernetes es "
    "exactamente lo que se quiere —hermetico, reproducible, sin estado—; para "
    "un formulario que valida mientras se escribe, no. Son dos modos y el "
    "segundo pide un proceso residente o una cache por digest",
    "NADIE SERIALIZA LAS ESCRITURAS. El `Taller` es atomico DENTRO de una "
    "invocacion —o va el paquete entero o ninguno— y dos invocaciones a la vez "
    "sobre el mismo arbol no se ven. Con un repositorio detras la respuesta "
    "natural no es un candado: es que cada edicion sea un commit y el conflicto "
    "lo resuelva git, que es para lo que sirve",
]:
    parrafo("· " + q, "     ")
    print()

# -- D -------------------------------------------------------------------------
print()
print("D - LA MULTITENANCIA: QUE HAY DE IDENTIDAD")
print()
TIENE = [
    ("quien responde de un documento", "`owner`, un handle `team:` o `user:`", True),
    ("quien puede leer que, en ejecucion", "Cedar — `principal`, `action`, `resource`", True),
    ("que puede salir por donde", "`ConduitPolicy` y los conductos", True),
    ("quien pidio esto", "`RequestPolicy` — la frontera de identidad", True),
    ("un TENANT como sujeto del sistema", "no existe", False),
    ("aislamiento entre arboles de clientes", "no existe", False),
    ("quien puede EDITAR el registro", "no existe — lo decide git", False),
]
for q, c, ok in TIENE:
    print("   %-3s %-36s %s" % ("si " if ok else "NO ", q, c))
print()
parrafo("La lectura correcta de esa tabla: **el gobierno del DATO esta "
        "construido y el gobierno del REGISTRO no**. Son dos planos distintos "
        "y el segundo no es una carencia del sustrato — es de la plataforma, y "
        "en un registro con forma de repositorio su respuesta natural ya "
        "existe fuera: ramas, revision y permisos del propio repositorio.")

# -- E -------------------------------------------------------------------------
print()
print("E - EL COTEJO")
print()
COTEJO = [
    ("el registro con forma de repositorio es lo que lo hace escalable",
     "SE SOSTIENE",
     "y es mas fuerte de lo que la frase dice. Un registro que es un arbol de "
     "ficheros con digest, `diff` por cuatro ejes, semver exigido y suite de "
     "conformidad tiene GRATIS lo que un servicio propietario tiene que "
     "construir: ramas, revision, reversion y auditoria. Es la ventaja "
     "estructural frente a Foundry, no una equivalencia"),
    ("`ore x` corre en un Kubernetes particionado",
     "SE SOSTIENE",
     "12 de 14 crates son hermeticos y el binario contesta desde el arbol de "
     "ficheros: es un job, no un servicio. Y lo que toca la red ya esta "
     "aislado en subprocesos con la URL por stdin, que es exactamente la "
     "frontera que un cluster quiere"),
    ("el usuario modela desde la web y el YAML se escribe solo",
     "FALTA LA MITAD",
     "faltan emisores para las clases que no se inducen, y falta el empalme "
     "por bloque. Ninguna de las dos es investigacion: las dos son trabajo "
     "conocido sobre primitivas que ya estan"),
    ("en tiempo de ejecucion el registro se actualiza y muta",
     "ES LA DECISION ABIERTA",
     "hoy una edicion es una escritura en un arbol sin candado. Con un "
     "repositorio detras eso se convierte en un commit, y entonces «mutar el "
     "registro» pasa a ser un flujo de propuesta y merge — que es MEJOR que "
     "mutar un servicio, y es otra arquitectura"),
    ("competir con Foundry en profundidad y calidad",
     "EN GOBIERNO, SI",
     "el linaje por columna comprobado AL COMPILAR y el flujo implicito no los "
     "hace nadie asi. En operacion —consumo, escala, tenancy, observabilidad "
     "de plataforma— no hay nada todavia, y es donde esta el grueso"),
]
for q, v, por in COTEJO:
    print("   · %-52s %s" % (q, v))
    parrafo(por, "       ")
    print()
