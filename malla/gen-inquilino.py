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
    "41-el-cofre.yaml",
    "42-el-arbol.yaml",
    "43-la-entrada.yaml",
    "50-jwks.yaml",
]

# ⛔⛔ LO QUE SE DEJA FUERA A PROPOSITO, Y CON SU MOTIVO ESCRITO.
#
# La comprobacion ⑥ exige que cada YAML de `malla/` tenga dueño. Sin esta tabla
# habria dos formas de no tenerlo —olvidarlo y decidirlo— y se verian igual.
#
# ⭐ Que excluir sea ESCRIBIR AQUI UNA FRASE es la mitad del valor: quien lo
#   haga tiene que decir por que, y el que venga detras lo lee en vez de
#   deducirlo de una ausencia.
FUERA = {
    "61-realms.yaml": (
        "Los tres `KeycloakRealmImport` vivos difieren del fichero, asi que "
        "aplicarlos les sube la `generation` y el operador REIMPORTA los realms. "
        "Y el realm vivo lleva cosas que el fichero no describe — `ore-agente` en "
        "`rubix-dev` se creo a mano con `kcadm`, y su secreto vive en el `Secret` "
        "`idp-agente`. Entra cuando fichero y realms digan lo mismo."
    ),
}

# ⛔ LOS QUE NOMBRAN INQUILINOS A PROPOSITO, Y NO SON PLANTILLAS.
#
# La ⑤ existe para cazar un `t-demo` olvidado en un manifiesto que se renderiza
# para otro cliente. Pero hay un fichero cuyo TRABAJO es nombrar inquilinos:
#
#   `13-…` es EL ENGANCHE. Lleva un par `GitRepository`+`Kustomization` por
#   inquilino, escritos a mano, y vive en `malla/` a proposito — es la parte que
#   decide QUE SE OBEDECE, y si viviera dentro de lo obedecido, quien escribiera
#   alli cambiaria a que apunta el agente.
#
# ⚠️ Y hasta hoy pasaba la ⑤ POR SUERTE: nombraba al inquilino como
#   `inquilino-demo`, que no contiene `t-demo`. Al mudar el compartimento a la
#   forja la URL pasó a ser `…/t-demo/compartimento.git` y salto. La regla no
#   habia cambiado — lo que habia cambiado era que por fin la tocaba.
NOMBRAN_INQUILINOS = {
    "13-el-inquilino-reconciliado.yaml":
        "es el enganche: un par por inquilino, escrito a mano y no renderizado.",
}

MODELO = "demo"

# ⛔ Como PALABRA, no como subcadena. La primera version buscaba `demo` en
#   crudo y salto con «de**mo**strado» en un comentario — un aviso que se
#   dispara por una palabra de la prosa ensena a ignorarlo, y entonces deja de
#   avisar del `t-demo` que si importa.
#
# ⭐ Las lindes son «no una letra», asi que `t-demo`, `cq-demo`, `cofre-demo@`
#   y `tenant: demo` siguen cazandose — todos los sitios donde el nombre es un
#   VALOR y no una silaba.
SUELTO = re.compile(r"(?<![A-Za-z])%s(?![A-Za-z])" % MODELO)


def render(nombre, arbol=None, entrada=None):
    """La plantilla con las sustituciones hechas. `arbol` es `<propietario>/<repo>`
    tal como lo guarda `iam.organizacion.arbol`; por defecto, lo que deriva
    `fundar`.

    ⛔ El orden de las sustituciones no es indiferente: el árbol se sustituye
    ANTES que el namespace, porque `t-demo/ontologia` contiene `t-demo`. Al
    revés, un árbol propio —`acme-corp/ontologia`— nunca llegaría a escribirse.
    """
    arbol = arbol or "t-%s/ontologia" % nombre
    # La puerta. Por defecto el subdominio nuestro, que es lo que el `Gateway`
    # compartido sirve sin coste por cliente (E6, opcion `b`). El que traiga su
    # dominio lo dice, y entonces el certificado depende de SU DNS.
    entrada = entrada or "%s.ore.paladio.io" % nombre
    salida = {}
    for f in PLANTILLAS:
        t = (MALLA / f).read_text(encoding="utf-8")
        t = t.replace("t-%s/ontologia" % MODELO, arbol)
        # ⛔ Y LA ENTRADA TAMBIEN ANTES, por el mismo argumento: es un valor
        #   ENTERO que lleva el nombre dentro. Una entrada propia
        #   —`ontologia.acme.com`— no comparte ni una letra con el derivado, asi
        #   que tiene que sustituirse la cadena completa o no se escribe nunca.
        t = t.replace("%s.ore.paladio.io" % MODELO, entrada)
        t = t.replace("t-%s" % MODELO, "t-%s" % nombre)
        t = t.replace("cq-%s" % MODELO, "cq-%s" % nombre)
        t = t.replace("serve-%s" % MODELO, "serve-%s" % nombre)
        # ⛔ Y la cuenta de Google del custodio, que lleva el inquilino dentro
        #   por necesidad: es UNA por inquilino, porque su permiso alcanza UNA
        #   clave. Compartirla —como hace `ore-driver@`— seria dar la llave de
        #   uno a los demas.
        t = t.replace("cofre-%s" % MODELO, "cofre-%s" % nombre)
        t = t.replace("ore.dev/tenant: %s" % MODELO, "ore.dev/tenant: %s" % nombre)
        # ⛔⛔ EL NOMBRE DE LA ORGANIZACION EN `ore init`, y no es cosmetico:
        #   `metadata.name` del manifiesto es lo que prefija cada
        #   `connectionEnv`. Medido — con `--name prueba` sale
        #   `PRUEBA_CRM_PROD_URL`; sin el, `CRM_PROD_URL` a secas, y dos
        #   inquilinos pisarian la misma variable.
        #
        # ⚠️ Va sustituido EXPLICITAMENTE y no por la via general, porque `demo`
        #   suelto es justo lo que la comprobacion ② prohibe. Si algun dia esta
        #   linea se cae, ② lo caza: el nombre del modelo no sobrevive.
        t = t.replace("--name %s" % MODELO, "--name %s" % nombre)
        # Y el mensaje del primer commit, que tambien lo lleva.
        t = t.replace("organizacion %s" % MODELO, "organizacion %s" % nombre)
        t = t.replace("arbol de \\`%s\\`" % MODELO, "arbol de \\`%s\\`" % nombre)
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
        quedan = [(i + 1, l.strip())
                  for i, l in enumerate(t.splitlines()) if SUELTO.search(l)]
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
        if f.name in NOMBRAN_INQUILINOS:
            print("     ⚠️ `%s` nombra inquilinos — %s"
                  % (f.name, NOMBRAN_INQUILINOS[f.name]))
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

    # ── ⑥ Y QUE NINGUN FICHERO SE QUEDE SIN QUIEN LO APLIQUE ──────────────
    #
    # ⭐⭐ La hermana de la ⑤, y es la misma caminata con la pregunta al reves.
    #   Desde que `malla/` esta bajo Flux, cada YAML de aqui pertenece a
    #   EXACTAMENTE UNA de tres listas:
    #
    #     PLANTILLAS            lo del inquilino. Lo aplica `inquilino-<org>`
    #     kustomization.yaml    lo de la plataforma. Lo aplica `malla`
    #     `9x-`                 pruebas. Las corre una persona, a mano
    #
    #   ⛔ Un fichero en NINGUNA no da un error: no lo aplica nadie, y quien lo
    #     escriba creera que si. Es el mismo fallo silencioso que la ⑤ caza del
    #     otro lado, y sin esto la unica forma de notarlo es que algo no exista
    #     el dia que haga falta.
    #
    #   ⛔⛔ Y un fichero en DOS es peor: dos `Kustomization` gobernando el
    #     mismo objeto no fallan — se lo escriben por turnos. Una plantilla que
    #     alguien edite pensando que edita una plantilla se aplicaria a un
    #     cliente.
    #
    # ⚠️ Se lee el fichero a mano y no con un analizador de YAML: `pyyaml` no
    #   esta garantizado aqui, y lo que hace falta es la lista de `resources`,
    #   que son lineas `  - <fichero>`.
    kfile = MALLA / "kustomization.yaml"
    if not kfile.exists():
        fallos.append("no hay `malla/kustomization.yaml`: Flux no sabria que aplicar")
    else:
        listados = set(
            re.findall(r"^\s*-\s+(\S+\.yaml)\s*$", kfile.read_text(encoding="utf-8"), re.M)
        )
        for f in sorted(MALLA.glob("*.yaml")):
            if f.name == "kustomization.yaml":
                continue
            plantilla, plataforma, prueba = (
                f.name in PLANTILLAS, f.name in listados, f.name[0] == "9",
            )
            if f.name in FUERA:
                # Fuera a proposito. Se dice, y con su motivo: una exclusion
                # muda no se distingue de un olvido.
                if plataforma:
                    fallos.append(
                        "`%s` esta en `FUERA` y tambien en `kustomization.yaml`"
                        % f.name)
                continue
            if plantilla and plataforma:
                fallos.append(
                    "`%s` esta en `PLANTILLAS` **y** en `kustomization.yaml`: dos "
                    "`Kustomization` gobernarian el mismo objeto" % f.name)
            elif not (plantilla or plataforma or prueba):
                fallos.append(
                    "`%s` no esta en `PLANTILLAS`, ni en `kustomization.yaml`, ni es "
                    "una prueba `9x-`: NO LO APLICA NADIE" % f.name)
        for n in sorted(listados - {p.name for p in MALLA.glob("*.yaml")}):
            fallos.append("`kustomization.yaml` lista `%s`, que no existe" % n)
    for n, porque in sorted(FUERA.items()):
        print("     ⚠️ `%s` fuera a proposito — %s" % (n, porque[:58] + "…"))
    print("  ⑥ cada YAML de `malla/` tiene exactamente un dueño que lo aplica")

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
    CON_VALOR = ("--arbol", "--entrada", "--a")
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

    hecho = render(nombre, valor("--arbol"), valor("--entrada"))
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
