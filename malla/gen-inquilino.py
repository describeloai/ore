# -*- coding: utf-8 -*-
"""Los manifiestos de UN inquilino, a partir de la plantilla.

    python malla/gen-inquilino.py acme
    python malla/gen-inquilino.py acme --arbol acme-corp/ontologia --a /tmp/acme
    python malla/gen-inquilino.py --comprobar

E1 de la `0022`. **No aplica nada y no necesita ninguna credencial**: es texto que
entra y texto que sale. Lo que haga falta hacer con lo que salga —empujarlo a un
repositorio, aplicarlo— es de otras etapas y de otra pieza.

── ⭐⭐ Por qué la plantilla NO lleva marcadores ────────────────────────────

Lo obvio sería escribir `{{tenant}}` en los YAML y rellenarlo aquí. No se hace, y
el motivo está en `ci.yml`: los manifiestos se aplican a mano y tienen que seguir
diciendo la verdad —

    «para que `kubectl apply -f malla/…` a mano siga diciendo la verdad»

⇒ Con marcadores, `malla/` dejaría de ser aplicable y pasaría a ser una fuente
que hay que compilar antes de mirar. Con `demo` dentro, **la plantilla es una
instancia válida**, y de ahí sale la primera comprobación de este generador:
renderizar `demo` tiene que devolver el fichero **byte a byte**. Si no, el
generador no está sustituyendo lo que cree.

── ⭐ Y la comprobación que de verdad importa ──────────────────────────────

Renderizar para otro nombre y exigir que **la palabra `demo` no aparezca en
ningún sitio de la salida**.

Es la que caza el fallo caro: un `t-demo` olvidado en un manifiesto de otro
cliente no da error — da un `ore-serve` de `acme` clonando el árbol de `demo`.
Ese fallo no aparece hasta que hay dos inquilinos, y para entonces ya pasó.

⚠️ El precio, dicho: se sustituye TAMBIÉN dentro de los comentarios. Una nota que
diga «el de `t-demo` es un árbol de desarrollo» acabará diciendo `t-acme`, que ya
no es exactamente lo que se midió. Se acepta porque la alternativa —distinguir
prosa de valor— es un analizador que puede equivocarse, y equivocarse aquí es
justo lo que no se puede.
"""
import pathlib
import re
import sys

try:
    sys.stdout.reconfigure(encoding="utf-8")
    # ⚠️ Y el de errores tambien: los avisos salen por ahi, y en una consola
    #    cp1252 se imprimian como `⚠` en vez de como un simbolo.
    sys.stderr.reconfigure(encoding="utf-8")
except AttributeError:
    pass

MALLA = pathlib.Path(__file__).resolve().parent

# ⛔ Los ficheros que constituyen a un inquilino, y sólo ésos. Los `9x-` son
#   pruebas y los de sistema —la forja, el IdP, Kueue— no se multiplican.
#
# ⚠️ `20-driver.yaml` está aquí y no se veía: es entero del inquilino —la cuenta
#   del driver y por dónde puede salir—, viviendo en un fichero que parecía de
#   sistema por el número. Lo encontró la comprobación ⑤.
#
# ⛔ Y arrastra una llamada que ningún YAML hace: esa cuenta lleva la anotación
#   de Workload Identity a `ore-driver@…`, y ese permiso se concede al PAR
#   `(namespace, cuenta)`. Un inquilino nuevo necesita su propio enlace —
#
#     gcloud iam service-accounts add-iam-policy-binding ore-driver@… \
#       --role=roles/iam.workloadIdentityUser \
#       --member="serviceAccount:<proyecto>.svc.id.goog[t-<nombre>/driver]"
#
#   — o su driver no podrá autenticarse contra Google, y el síntoma será un
#   permiso denegado que manda a mirar los roles y no el enlace. Va a la E4.
PLANTILLAS = [
    "11-el-inquilino.yaml",
    "20-driver.yaml",
    "40-ore-serve.yaml",
    "50-jwks.yaml",
]

MODELO = "demo"


def render(nombre, arbol=None):
    """La plantilla con las sustituciones hechas. `arbol` es `<propietario>/<repo>`
    tal como lo guarda `iam.organizacion.arbol`; por defecto, lo que deriva
    `fundar`.

    ⛔ El orden de las sustituciones no es indiferente: el árbol se sustituye
    ANTES que el namespace, porque `t-demo/ontologia` contiene `t-demo`. Al
    revés, un árbol propio —`acme-corp/ontologia`— nunca llegaría a escribirse.
    """
    arbol = arbol or "t-%s/ontologia" % nombre
    salida = {}
    for f in PLANTILLAS:
        t = (MALLA / f).read_text(encoding="utf-8")
        t = t.replace("t-%s/ontologia" % MODELO, arbol)
        t = t.replace("t-%s" % MODELO, "t-%s" % nombre)
        t = t.replace("cq-%s" % MODELO, "cq-%s" % nombre)
        t = t.replace("serve-%s" % MODELO, "serve-%s" % nombre)
        t = t.replace("ore.dev/tenant: %s" % MODELO, "ore.dev/tenant: %s" % nombre)
        salida[f] = t
    return salida


def nombre_valido(s):
    """El mismo alfabeto que la `017` exige para el primer segmento del árbol.
    Se comprueba aquí porque de este nombre salen un namespace y una cola, y los
    dos tienen que ser nombres de Kubernetes válidos."""
    return bool(re.match(r"^[a-z0-9][a-z0-9-]{0,60}$", s))


# ══════════════════════════════════════════════════════════════════════════
def comprobar():
    fallos = []

    # ── ① La plantilla es una instancia: renderizar `demo` es la identidad ──
    for f, t in render(MODELO).items():
        if t != (MALLA / f).read_text(encoding="utf-8"):
            fallos.append("`%s`: renderizar `demo` NO devuelve el fichero" % f)
    print("  ① renderizar `%s` devuelve la plantilla, byte a byte" % MODELO)

    # ── ② Para otro nombre, `demo` no sobrevive en ningún sitio ─────────────
    otro = "acme"
    hecho = render(otro)
    for f, t in hecho.items():
        quedan = [(i + 1, l.strip()) for i, l in enumerate(t.splitlines()) if MODELO in l]
        if quedan:
            fallos.append(
                "`%s`: quedan %d menciones de `%s` — la primera en la linea %d: %s"
                % (f, len(quedan), MODELO, quedan[0][0], quedan[0][1][:70]))
    print("  ⭐ ② renderizado para `%s`, la palabra `%s` no aparece" % (otro, MODELO))

    # ── ③ Y no se ha perdido nada por el camino ─────────────────────────────
    #
    # Contar es lo que separa «sustituyó» de «borró». Un `replace` con el
    # argumento cambiado dejaría la salida limpia de `demo` y vacía de todo lo
    # demás, y ② no lo notaría.
    for f in PLANTILLAS:
        origen = (MALLA / f).read_text(encoding="utf-8")
        if origen.count("t-%s" % MODELO) != hecho[f].count("t-%s" % otro):
            fallos.append("`%s`: el numero de namespaces no cuadra" % f)
        if origen.count("\n") != hecho[f].count("\n"):
            fallos.append("`%s`: el numero de lineas cambio" % f)
        if origen.count("\n---") != hecho[f].count("\n---"):
            fallos.append("`%s`: el numero de documentos cambio" % f)
    print("  ③ mismas lineas, mismos documentos y mismos namespaces")

    # ── ④ Y el árbol propio llega entero ────────────────────────────────────
    propio = render("acme", arbol="acme-corp/ontologia")["40-ore-serve.yaml"]
    if "acme-corp/ontologia.git" not in propio:
        fallos.append("un arbol propio no llega a `--forja`")
    if "t-acme/ontologia.git" in propio:
        fallos.append("`--forja` sigue apuntando al arbol derivado, no al propio")
    print("  ④ un arbol propio sustituye al derivado en `--forja`")

    # ── ⑤ Y que no haya aparecido un CUARTO fichero del inquilino ──────────
    #
    # ⭐ Es la comprobación que impide que esto envejezca en silencio. El día que
    #   alguien añada a `malla/` otro manifiesto con cosas del inquilino y no lo
    #   ponga en `PLANTILLAS`, renderizar para un cliente nuevo **no lo emitiría**
    #   — y el inquilino saldría a medias sin que nada fallara.
    #
    # Se miran los VALORES, no los comentarios: `31-copias-de-la-forja.yaml`
    # nombra `t-demo/ontologia` en su ejemplo de restauración, y eso es prosa.
    # Y los `9x-` quedan fuera porque son pruebas contra el inquilino modelo, no
    # partes de él.
    for f in sorted(MALLA.glob("*.yaml")):
        if f.name in PLANTILLAS or f.name[0] == "9":
            continue
        culpables = [
            (i + 1, l.strip())
            for i, l in enumerate(f.read_text(encoding="utf-8").splitlines())
            if ("t-%s" % MODELO) in l and not l.lstrip().startswith("#")
        ]
        if culpables:
            fallos.append(
                "`%s` habla del inquilino en un VALOR (linea %d) y no esta en "
                "`PLANTILLAS`: renderizar otro cliente lo dejaria fuera"
                % (f.name, culpables[0][0]))
    print("  ⑤ ningun otro manifiesto de `malla/` lleva el inquilino en un valor")

    if fallos:
        print("\n⛔ EL RENDERIZADOR MIENTE:")
        for x in fallos:
            print("   · " + x)
        return 1
    print("\n✓ la plantilla y el renderizador dicen lo mismo")
    return 0


# ══════════════════════════════════════════════════════════════════════════
def main(argv):
    if "--comprobar" in argv:
        return comprobar()

    # ⛔ Y hay que saltarse el VALOR de cada opción, no sólo la opción. Filtrar
    #   por `--` dejaba el destino de `--a` contado como si fuera el nombre del
    #   inquilino, así que `gen-inquilino.py acme --a /tmp/x` imprimía la ayuda
    #   —dos «nombres»— en vez de escribir nada. Un uso correcto contestado con
    #   la ayuda se lee como «lo he escrito mal», y manda a mirar el nombre.
    CON_VALOR = ("--arbol", "--a")
    libres, saltar = [], False
    for a in argv:
        if saltar:
            saltar = False
            continue
        if a in CON_VALOR:
            saltar = True
        elif not a.startswith("--"):
            libres.append(a)
    if len(libres) != 1:
        print(__doc__.split("\n\n")[0])
        print("\n  python malla/gen-inquilino.py <nombre> [--arbol P/R] [--a DIR]")
        print("  python malla/gen-inquilino.py --comprobar")
        return 64  # EX_USAGE

    nombre = libres[0]
    if not nombre_valido(nombre):
        print("✗ `%s` no sirve como nombre de inquilino." % nombre, file=sys.stderr)
        print("  Minuscula o digito, luego minusculas, digitos o `-`.", file=sys.stderr)
        print("  De aqui salen un namespace y una cola de Kubernetes.", file=sys.stderr)
        return 65  # EX_DATAERR
    if nombre == MODELO:
        print("⚠ `%s` es la plantilla: renderizarlo devuelve lo que ya hay." % MODELO,
              file=sys.stderr)

    def valor(que):
        return argv[argv.index(que) + 1] if que in argv and argv.index(que) + 1 < len(argv) else None

    hecho = render(nombre, valor("--arbol"))
    destino = valor("--a")
    if destino:
        d = pathlib.Path(destino)
        d.mkdir(parents=True, exist_ok=True)
        for f, t in hecho.items():
            (d / f).write_text(t, encoding="utf-8", newline="\n")
            print("  %s" % (d / f))
        # ⛔ Y se dice lo que NO ha salido de aqui, porque quien mire este
        #   directorio va a creer que tiene un inquilino entero.
        print("\n  ⚠ Falta lo que no cabe en un YAML: el repositorio en la forja,")
        print("    su usuario y su testigo, y el `Secret` del agente. Tres llamadas,")
        print("    no tres ficheros — ver la E4 de `docs/decisions/0022`.")
        return 0

    for f, t in hecho.items():
        sys.stdout.write(t)
        if not t.endswith("\n"):
            sys.stdout.write("\n")
        sys.stdout.write("---\n")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
