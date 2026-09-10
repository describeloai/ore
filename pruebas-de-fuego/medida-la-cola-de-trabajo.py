# -*- coding: utf-8 -*-
"""MEDIDA · partir el compartimento en dos: lo que GOBIERNA y lo que está POR HACER.

Lo que lo dispara, medido el 2026-09-10: se da de alta una fuente a las 19:24 y
el Job que lee el origen **no existe** hasta que el cron pase a las 20:17. Casi
una hora, y no por lentitud: por un hueco de diseño.

    ① el alta empuja al ARBOL
    ② el webhook de organizacion SI dispara
    ③ el `Receiver` reconcilia los `GitRepository` con `ore.dev/rol: agente`
    ④ …pero esos apuntan al COMPARTIMENTO, que no ha cambiado
    ⑤ Flux mira, ve lo mismo, y no hace nada

⭐⭐ Y lo obvio —que `ore-serve` rinda el Job en el mismo acto— es una ESCALADA
  DE PRIVILEGIO, porque el compartimento contiene su propio `Deployment`. Esta
  medida es sobre la salida a eso, y sobre lo que esa salida cuesta de verdad.

    uso:  python pruebas-de-fuego/medida-la-cola-de-trabajo.py
"""
import importlib.util
import pathlib
import re
import sys

try:
    sys.stdout.reconfigure(encoding="utf-8")
    sys.stderr.reconfigure(encoding="utf-8")
except AttributeError:
    pass

RAIZ = pathlib.Path(__file__).resolve().parent.parent
fallos = []


def leer(p):
    f = RAIZ / p
    return f.read_text(encoding="utf-8") if f.exists() else ""


def titulo(t):
    print("\n" + "═" * 74)
    print(t)
    print("═" * 74)


def cargar_gen():
    e = importlib.util.spec_from_file_location("gen", RAIZ / "malla" / "gen-inquilino.py")
    m = importlib.util.module_from_spec(e)
    e.loader.exec_module(m)
    return m


# ══════════════════════════════════════════════════════════════════════════
titulo("① LA LINEA DEL CORTE YA ESTA TRAZADA — en el renderizador")
# ══════════════════════════════════════════════════════════════════════════
#
# ⭐ Esto es lo que hace la propuesta barata: no hay que inventar la distincion,
#   solo darle dos repositorios. `gen-inquilino.py` ya emite dos categorias y las
#   nombra, y su comentario dice por que son distintas.
g = cargar_gen()
print("  PLANTILLAS · lo que GOBIERNA al inquilino, uno de cada:")
for f in g.PLANTILLAS:
    print("      %s" % f)
print("\n  POR_FUENTE · lo que esta POR HACER, uno por fuente pendiente:")
print("      %s" % g.POR_FUENTE)
if len(g.PLANTILLAS) < 5 or not g.POR_FUENTE:
    fallos.append("el renderizador ya no tiene dos categorias: esta medida habla de otro arbol")

print("""
  ⇒ Siete ficheros dicen COMO ES el inquilino —su namespace, su servidor, su
    cofre, su entrada, sus llaves— y uno dice QUE HAY QUE HACER. Son cosas de
    naturaleza distinta y hoy viven en el mismo repositorio, con el mismo
    escritor y el mismo permiso.""")

# ══════════════════════════════════════════════════════════════════════════
titulo("② POR QUE `ore-serve` NO PUEDE ESCRIBIR AHI HOY")
# ══════════════════════════════════════════════════════════════════════════
plantillas = {f: leer("malla/" + f) for f in g.PLANTILLAS}
tipos = {}
for f, t in plantillas.items():
    for k in re.findall(r"^kind:\s*(\w+)", t, re.M):
        tipos[k] = tipos.get(k, 0) + 1
print("  Lo que hay dentro del compartimento, por tipo:")
for k in sorted(tipos, key=lambda x: -tipos[x]):
    print("      %-22s x%d" % (k, tipos[k]))

peligrosos = [k for k in tipos if k in
              ("Deployment", "ServiceAccount", "NetworkPolicy", "Secret", "RoleBinding", "Role")]
print("""
  ⛔ Entre ellos: %s.

    Quien pueda escribir en ese repositorio reescribe el `Deployment` del propio
    `ore-serve`, sus politicas de red y a que cuenta corre. Darle escritura para
    que rinda un Job de catalogo le daria eso ADEMAS — y eso no es un permiso de
    mas: es que el gobernado pase a escribir su gobierno.""" % ", ".join(sorted(peligrosos)))
if "Deployment" not in tipos:
    fallos.append("el compartimento ya no lleva `Deployment`: el argumento de arriba no vale")

# ══════════════════════════════════════════════════════════════════════════
titulo("③ ⛔⛔ Y PARTIR EL REPOSITORIO **NO** QUITA LA ESCALADA")
# ══════════════════════════════════════════════════════════════════════════
#
# Este es el hallazgo que cambia el diseño, y no se ve hasta que se mira el
# enganche: un `Kustomization` de Flux sin `serviceAccountName` aplica con el
# `cluster-admin` del controlador.
enganche = leer("malla/13-el-inquilino-reconciliado.yaml")
tiene_sa = bool(re.search(r"^\s*serviceAccountName:", enganche, re.M))
print("  `13-el-inquilino-reconciliado.yaml` declara `serviceAccountName`: %s"
      % ("SI" if tiene_sa else "NO"))
nota = re.search(r"NO se le da `serviceAccountName`.*?(?=\n#\s*\n|\n---)", enganche, re.S)
if nota:
    print("  y su propio comentario lo dice:\n")
    for l in nota.group(0).split("\n")[:6]:
        print("      " + l.strip().lstrip("# ").rstrip())
if tiene_sa:
    fallos.append("`13-…` ya tiene `serviceAccountName`: esta medida describe un estado viejo")

print("""
  ⇒ Asi que un segundo repositorio con un segundo `Kustomization` **sin acotar**
    dejaria a `ore-serve` escribiendo manifiestos que Flux aplica como
    cluster-admin. Podria poner ahi un `Deployment`, un `RoleBinding` o un
    `Secret`, y la escalada volveria por la otra puerta.

  ⭐⭐ LUEGO (C) PURA NO SON DOS PIEZAS, SON TRES:

      ① un repositorio `t-<org>/trabajo`, escrito por `ore-serve`
      ② un `GitRepository` + `Kustomization` que lo obedece
      ③ una CUENTA ACOTADA que ese `Kustomization` impersona, y que solo puede
         crear `Job` en el namespace del inquilino

    Sin la ③ esto no es un arreglo de seguridad: es el mismo agujero con dos
    repositorios.""")

# ══════════════════════════════════════════════════════════════════════════
titulo("④ Y LA ③ CIERRA UNA DEUDA QUE LLEVA ABIERTA TODO EL DIA")
# ══════════════════════════════════════════════════════════════════════════
print("""  El `cluster-admin` de Flux es el punto ④ de la `0022` y sigue abierto. La
  razon escrita de que no se acotara es concreta:

      «este manifiesto CREA su propio `Namespace`, que es un recurso de cluster
       y no cabe en una cuenta acotada al namespace»

  ⭐ Y eso vale para el compartimento de infraestructura — que crea el
    `Namespace`— y **no vale para la cola de trabajo**, que solo crea `Job`
    dentro de un namespace que ya existe.

  ⇒ Partir en dos no solo da el disparo instantaneo: da el primer
    `Kustomization` de este arbol que PUEDE correr acotado, porque es el primero
    que no crea nada de ambito de cluster.""")

# ══════════════════════════════════════════════════════════════════════════
titulo("⑤ EL DISPARO, Y POR QUE SERA INSTANTANEO SIN NADA NUEVO")
# ══════════════════════════════════════════════════════════════════════════
prov = leer("malla/aprovisionar-inquilino.sh")
org = "por ORGANIZACION" in prov and "todavia no existen" in prov.replace("í", "i")
print("  El webhook de la forja es de ORGANIZACION, no de repositorio: %s"
      % ("confirmado en el aprovisionador" if org else "⚠️ no se encuentra la nota"))
if not org:
    fallos.append("el aprovisionador ya no describe el webhook como de organizacion")
print("""
  ⇒ Y su comentario dice que cubre «todos los repositorios del inquilino —
    incluidos los que todavia no existen». Asi que `t-<org>/trabajo` YA esta
    cubierto el dia que se cree: no hay que tocar el webhook, ni el `Receiver`,
    ni el aprovisionador.

  El camino entero, despues:

      alta  →  ore-serve escribe el arbol Y la cola  →  webhook  →  Receiver
            →  reconcilia el GitRepository de la cola  →  Flux crea el Job

  ⭐ Segundos, no una hora. Y ni un actor nuevo: el `Receiver` ya existe, nombra
    una CLASE por etiqueta —`ore.dev/rol: agente`— y el `GitRepository` nuevo
    entra en el con solo llevarla.""")

# ══════════════════════════════════════════════════════════════════════════
titulo("⑥ LO QUE ESTO NO ARREGLA, Y HAY QUE VIGILAR")
# ══════════════════════════════════════════════════════════════════════════
print("""  ⛔ `ore-serve` gana escritura sobre UN repositorio de la forja. Un `ore-serve`
    comprometido puede encolar Jobs de catalogo a voluntad — gastar cuota, leer
    origenes que ya tenia permitidos. Lo que NO puede es cambiar como se
    despliega, ni crear nada fuera de `Job`, ni tocar otro inquilino.

  ⚠️ Y ese limite lo pone la cuenta acotada, no el repositorio. Si algun dia
    alguien le quita el `serviceAccountName` a ese `Kustomization` «para
    depurar», la escalada vuelve entera y en silencio. Va con su comentario, y
    la comprobacion ⑥ de `gen-inquilino.py` es el sitio natural para exigirlo.

  ⚠️ El Job de catalogo lleva `ore.dev/rol: driver` y su `NetworkPolicy` sale a
    internet por 443 y 5432. Encolar Jobs es encolar salidas a la red — acotado
    por esos puertos y por el custodio, que sigue decidiendo si suelta la
    credencial. No cambia con esto, pero conviene tenerlo junto.

  ⛔ Y la cola de trabajo tendra `prune: true` como el compartimento. Un fichero
    que desaparece borra su Job. Para un Job terminado da igual; para uno
    corriendo, lo mata. Hay que decidirlo a conciencia, no heredarlo.""")

print("\n" + "═" * 74)
if fallos:
    print("⛔ LA MEDIDA NO SE SOSTIENE:")
    for f in fallos:
        print("   · " + f)
    sys.exit(1)
print("✓ medida coherente: (C) son TRES piezas, y la tercera cierra el ④ de la `0022`")
