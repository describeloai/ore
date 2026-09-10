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
import hashlib
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
# ✓ Y arrastraba una llamada que ningún YAML hace: el enlace de Workload
#   Identity, que se concede al PAR `(namespace, cuenta)`. Lo hace el
#   aprovisionador, y desde hoy sobre `ore-driver-<inquilino>@` — una cuenta POR
#   INQUILINO, no la compartida de antes.
#
# ⛔ Lo forzó el catálogo: el Job que lee un origen tiene que EMPUJAR el
#   resultado al árbol de su inquilino, o sea leer el testigo de la forja de ese
#   inquilino. Con una cuenta compartida, dárselo a uno se lo daba a todos — que
#   es exactamente el patrón que el cofre rechazó con su llave. El driver era la
#   última excepción viva.
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
    "papel-del-aprovisionador.yaml": (
        "NO es un manifiesto de Kubernetes: es la definicion de un papel de IAM "
        "de Google, y se aplica con `gcloud iam roles create`. Vive aqui porque "
        "es parte del mismo compartimento que el Job de al lado y separarlos "
        "haria que uno se moviera sin el otro. ⚠️ Y arrastra que el papel NO "
        "esta bajo GitOps: cambiarlo es una llamada a mano."
    ),
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

# ⛔⛔ LA QUE SE RINDE N VECES, Y ES UNA CATEGORIA NUEVA.
#
# Las de `PLANTILLAS` salen UNA por inquilino. Esta sale una POR FUENTE PENDIENTE
# — la primera del renderizador que se multiplica dentro de un compartimento.
#
# ⇒ Y es lo que convierte esto en el ciclo de vida del producto y no solo en su
#   montaje: el arbol declara una fuente, el reconciliador ve que no tiene
#   paquete, y escribe aqui un Job. Flux lo crea. Nadie despacha nada.
#
# ⚠️ El nombre del fichero lleva la fuente dentro, asi que dos fuentes no se
#   pisan y quitar una del arbol hace desaparecer SU Job — que con `prune: true`
#   en el compartimento es exactamente lo que debe pasar.
POR_FUENTE = "44-el-catalogo.yaml"

MODELO = "demo"

# La fuente del fichero modelo, como `demo` es el inquilino modelo. Renderizar
# `bq` tiene que devolver la plantilla byte a byte.
FUENTE_MODELO = "bq"

# ⛔ Como PALABRA, no como subcadena. La primera version buscaba `demo` en
#   crudo y salto con «de**mo**strado» en un comentario — un aviso que se
#   dispara por una palabra de la prosa ensena a ignorarlo, y entonces deja de
#   avisar del `t-demo` que si importa.
#
# ⭐ Las lindes son «no una letra», asi que `t-demo`, `cq-demo`, `cofre-demo@`
#   y `tenant: demo` siguen cazandose — todos los sitios donde el nombre es un
#   VALOR y no una silaba.
SUELTO = re.compile(r"(?<![A-Za-z])%s(?![A-Za-z])" % MODELO)

# Los ocho digitos que el nombre de un Job de catalogo lleva detras. La ①
# los normaliza para poder comparar; la ⑨ es quien exige que esten.
RESUMEN = re.compile(r"-[0-9a-f]{8}\b")


def render(nombre, arbol=None, entrada=None, fuentes=()):
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
        # ✓ Y el driver, que desde hoy TAMBIEN tiene cuenta propia. Era la
        #   ultima que se compartia entre inquilinos, y la forzo el catalogo:
        #   el Job que lee un origen tiene que empujar el resultado al arbol de
        #   SU inquilino, y con una cuenta compartida darselo a uno se lo daba
        #   a todos.
        t = t.replace("driver-%s" % MODELO, "driver-%s" % nombre)
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
        # ⛔ Y la organizacion que `ore-serve` le dice al custodio. Va en su
        #   propia linea, asi que se sustituye la PAREJA entera: un `- demo`
        #   suelto no lo caza ninguna de las reglas de arriba, y la ② lo
        #   destapo a la primera.
        pareja = "- --organizacion\n            - %s"
        t = t.replace(pareja % MODELO, pareja % nombre)
        # Y el mensaje del primer commit, que tambien lo lleva.
        t = t.replace("organizacion %s" % MODELO, "organizacion %s" % nombre)
        t = t.replace("arbol de \\`%s\\`" % MODELO, "arbol de \\`%s\\`" % nombre)
        salida[f] = t

    # ── Y una por cada fuente pendiente ───────────────────────────────────
    #
    # ⚠️ Las mismas sustituciones del inquilino y ADEMAS la fuente. El orden
    #   importa por lo de siempre: `catalogo-bq` contiene `bq`, asi que el
    #   nombre del Job se sustituye con la cadena entera y no por partes.
    plantilla = (MALLA / POR_FUENTE).read_text(encoding="utf-8")
    vistos = {}
    for fuente in fuentes:
        # ⛔⛔ EL NOMBRE DE LA FUENTE LO ESCRIBE EL CLIENTE, Y NO ES UN NOMBRE
        #   DE KUBERNETES. Esto iba directo a `metadata.name` y rompio a `demo`
        #   entero: su arbol trae fuentes como `postgresql_20260909_074550`, y
        #
        #     Job.batch "catalogo-postgresql_20260909_074550" is invalid:
        #     metadata.name: a lowercase RFC 1123 subdomain must consist of…
        #
        #   dejo su `Kustomization` en `False` — o sea, NADA de ese inquilino se
        #   aplicaba, ni lo que no tenia que ver con el catalogo. Un nombre
        #   ajeno tumbando un compartimento entero.
        #
        # ⚠️ Y no lo vio venir nadie porque hasta hoy solo se rendia a mano,
        #   para inquilinos con nombres limpios. Converger a todos es lo que lo
        #   destapo — que es exactamente para lo que sirve converger a todos.
        #
        # ⭐ El VALOR sigue siendo el nombre de verdad: es lo que `ore source
        #   catalog` recibe y lo que nombra al secreto `fuente-<n>` en el cofre.
        #   Lo unico que se limpia es el nombre del OBJETO.
        obj = nombre_de_objeto(fuente)
        if obj in vistos:
            raise ValueError(
                "las fuentes `%s` y `%s` dan el mismo nombre de objeto `%s`: "
                "hay que desambiguarlas en el arbol" % (vistos[obj], fuente, obj))
        vistos[obj] = fuente
        t = plantilla
        t = t.replace('value: "%s"' % FUENTE_MODELO, 'value: "%s"' % fuente)
        t = t.replace('value: "%s"' % MODELO, 'value: "%s"' % nombre)
        t = t.replace("t-%s/ontologia" % MODELO, arbol)
        t = t.replace("t-%s" % MODELO, "t-%s" % nombre)
        t = t.replace("ore.dev/tenant: %s" % MODELO, "ore.dev/tenant: %s" % nombre)
        # ── ⭐⭐ Y EL NOMBRE, EL ULTIMO, CON EL RESUMEN DE TODO LO DEMAS ──
        #
        # Un Job es inmutable: mismo nombre y distinto contenido es un
        # «field is immutable» al aplicar. La plantilla llevaba
        # `ssa: IfNotPresent` para esquivarlo, y esquivarlo costaba que un Job
        # fallado no se reintentara nunca y que una plantilla nueva no llegara
        # al que ya existia. Ver la cabecera de `44-el-catalogo.yaml`.
        #
        # ⇒ Con el nombre derivado del contenido, «mismo nombre» implica «mismo
        #   contenido», y el conflicto deja de poder existir. La sustitucion va
        #   LA ULTIMA porque el resumen se toma de todo lo anterior.
        h = hashlib.sha256(t.encode("utf-8")).hexdigest()[:8]
        t = t.replace("catalogo-%s-00000000" % FUENTE_MODELO,
                      "catalogo-%s-%s" % (obj, h))
        salida["44-el-catalogo-%s.yaml" % obj] = t
    return salida


def nombre_de_objeto(s):
    """De un nombre de FUENTE al nombre de un objeto de Kubernetes.

    El de la fuente lo escribe el cliente en su arbol y puede traer mayusculas,
    guiones bajos, puntos y acentos; el del objeto tiene que ser un subdominio
    RFC 1123. Se limpia, no se rechaza: negarle a un cliente un nombre de
    fuente valido porque a Kubernetes no le gusta seria trasladarle una
    restriccion nuestra.

    ⚠️ Cortado a 30, y la cuenta es esta: `catalogo-` son 9, el resumen del
      contenido anade `-` y 8, y un Job le pone a sus pods otro sufijo de 6.
      9+30+1+8+6 = 54, con margen bajo el limite duro de 63.
    """
    n = re.sub(r"[^a-z0-9-]+", "-", s.lower()).strip("-")[:30].strip("-")
    return n or "sin-nombre"


def nombre_valido(s):
    """El mismo alfabeto que la `017` exige para el primer segmento del árbol.
    Se comprueba aquí porque de este nombre salen un namespace y una cola, y los
    dos tienen que ser nombres de Kubernetes válidos."""
    return bool(re.match(r"^[a-z0-9][a-z0-9-]{0,60}$", s))


# ══════════════════════════════════════════════════════════════════════════
def comprobar_plantillas():
    """Las que sólo miran las PLANTILLAS y lo que sale de ellas.

    ⭐⭐ Estan separadas de la ⑤ y la ⑥ porque son las unicas que **puede
      correr el que converge**. El `CronJob` que rinde los compartimentos monta
      un `ConfigMap` con los diez ficheros que rinde, no con `malla/` entero, y
      la ⑤ y la ⑥ caminan el directorio: alli dentro pasarian en verde por no
      encontrar nada que mirar, que es la peor forma de pasar.

    ⇒ Lo que queda aqui es exactamente lo que protege a un inquilino de una
      plantilla mala. Lo que queda fuera protege al REPOSITORIO de un
      despiste, y eso lo comprueba CI, que si tiene el arbol delante.
    """
    fallos = []

    # ── ① La plantilla es una instancia: renderizar `demo` es la identidad ──
    # ⭐ Y con la fuente modelo, para que la que se rinde N veces entre tambien
    #   en la identidad. Sin esto podria derivar sin que nadie lo notara.
    # ⚠️ Con UNA excepcion, y es la unica que tiene esta comprobacion: el
    #   nombre del Job de catalogo lleva el resumen de su propio contenido, asi
    #   que no puede coincidir con el `00000000` de la plantilla. Se normalizan
    #   LOS DOS lados a `00000000` y se compara el resto byte a byte.
    #
    # ⛔ Y es una excepcion de VERDAD, no un descuido: la ① deja de vigilar esos
    #   ocho caracteres. Quien los vigila es la ⑨, que exige que el nombre
    #   rendido tenga exactamente la forma `catalogo-<fuente>-<8 hex>`.
    for f, t in render(MODELO, fuentes=[FUENTE_MODELO]).items():
        origen = MALLA / (POR_FUENTE if f.startswith("44-") else f)
        a, b = t, origen.read_text(encoding="utf-8")
        if f.startswith("44-"):
            a, b = (RESUMEN.sub("-00000000", x) for x in (a, b))
        if a != b:
            fallos.append("`%s`: renderizar `demo` NO devuelve el fichero" % f)
    print("  ① renderizar `%s` devuelve la plantilla, byte a byte" % MODELO)

    # ── ② Para otro nombre, `demo` no sobrevive en ningún sitio ─────────────
    otro = "acme"
    # ⛔ CON UNA FUENTE, o la que se rinde N veces no entra en esta
    #   comprobacion y `demo` puede sobrevivir ahi sin que nadie lo vea. Paso:
    #   `44-el-catalogo.yaml` llevaba `/organizaciones/demo/` y la ② dijo que
    #   todo estaba bien, porque con `fuentes=()` ese fichero no se emite.
    hecho = render(otro, fuentes=[FUENTE_MODELO])
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

    # ── ⑦ Y QUE LO RENDIDO SEA YAML ────────────────────────────────────────
    #
    # ⛔⛔ Faltaba, y se pagó: el 2026-09-09 se empujó `44-el-catalogo.yaml`
    #   roto —un `python3 -c` con saltos deja sus lineas sin indentar y eso
    #   termina el bloque literal— y las seis comprobaciones dijeron que todo
    #   estaba bien. La ① compara lo rendido con el fichero byte a byte, asi
    #   que un fichero roto se compara consigo mismo y pasa.
    #
    # ⚠️ Aquello no llego a nadie porque nada rendia automaticamente. Bajo el
    #   `CronJob` que converge, un fichero asi llega a TODOS los inquilinos en
    #   una hora, y con `prune: true`. Esta comprobacion es la condicion para
    #   que ese `CronJob` pueda existir.
    yaml_hay = True
    try:
        import yaml
    except ImportError:
        yaml_hay = False
        # ⛔ Y NO se salta en silencio. Una comprobacion ausente y una
        #   comprobacion que pasa se leen igual en un registro — y esa
        #   confusion es exactamente la que dejo pasar el fichero roto.
        fallos.append(
            "no hay analizador de YAML aqui: la ⑦ NO se ha hecho. "
            "`apk add py3-yaml` / `pip install pyyaml`")
    else:
        for f, t in sorted(hecho.items()):
            try:
                list(yaml.safe_load_all(t))
            except Exception as e:
                fallos.append("`%s`: lo rendido NO es YAML — %s"
                              % (f, str(e).replace("\n", " ")[:120]))
    print("  ⭐ ⑦ cada fichero rendido se analiza como YAML de verdad")

    # ── ⑨ Y QUE EL NOMBRE DE UNA FUENTE AJENA NO TUMBE UN COMPARTIMENTO ────
    #
    # ⛔⛔ Esto rompio a `demo` de verdad el 2026-09-10. Su arbol trae fuentes
    #   como `postgresql_20260909_074550`, el nombre iba directo a
    #   `metadata.name`, y el resultado fue:
    #
    #     Job.batch "catalogo-postgresql_20260909_074550" is invalid:
    #     metadata.name: a lowercase RFC 1123 subdomain must consist of…
    #
    #   ⇒ El `Kustomization` de `demo` entero en `False`. No fallo el Job del
    #     catalogo: fallo el COMPARTIMENTO, porque un `Kustomization` valida
    #     todo antes de aplicar nada. Un nombre que escribio un cliente en su
    #     arbol dejando sin reconciliar a su inquilino completo.
    #
    # ⚠️ Y la ⑦ no lo veia: aquel YAML era perfectamente valido. Lo invalido no
    #   era el documento, era el NOMBRE — y la ⑦ solo analizaba.
    #
    # ⭐ Se rinde con un nombre hostil A PROPOSITO. Las demas comprobaciones
    #   usan `bq`, que es limpio, asi que ninguna podia tropezar con esto: hay
    #   que traer la suciedad de fuera para encontrarla.
    SUCIA = "PostgreSQL_2026-09-09 (Ventas).v2"
    RFC1123 = re.compile(r"^[a-z0-9]([-a-z0-9.]*[a-z0-9])?$")
    try:
        sucio = render(otro, fuentes=[SUCIA])
    except ValueError as e:
        fallos.append("una fuente con nombre sucio revienta el renderizador: %s" % e)
        sucio = {}
    if not yaml_hay:
        pass          # ya se dijo en la ⑦; no se repite el mismo fallo dos veces
    else:
        for f, t in sorted(sucio.items()):
            # Solo los del catalogo llevan el nombre de la fuente dentro;
            # los demas se llaman siempre igual.
            if f.startswith("44-el-catalogo-") and not RFC1123.match(
                    f[len("44-el-catalogo-"):-len(".yaml")]):
                fallos.append("`%s`: el nombre del fichero sale sucio" % f)
            for d in yaml.safe_load_all(t):
                if not isinstance(d, dict):
                    continue
                n = (d.get("metadata") or {}).get("name")
                if n is not None and not RFC1123.match(str(n)):
                    fallos.append(
                        "`%s`: %s se llama `%s`, y Kubernetes lo rechaza — eso "
                        "tumba el `Kustomization` del inquilino ENTERO"
                        % (f, d.get("kind"), n))
                # ⭐ Y el Job, ademas, con el resumen detras. La ① dejo de
                #   vigilar estos ocho caracteres al normalizarlos para poder
                #   comparar; se vigilan aqui, o no los vigila nadie.
                if (f.startswith("44-el-catalogo-")
                        and d.get("kind") == "Job") and not re.match(
                        r"^catalogo-[a-z0-9][a-z0-9-]*-[0-9a-f]{8}$", str(n)):
                    fallos.append(
                        "`%s`: el Job se llama `%s` y no `catalogo-<fuente>-<8 "
                        "hex>` — sin el resumen, dos contenidos distintos "
                        "compartirian nombre y volveria el `field is immutable`"
                        % (f, n))
    print("  ⭐ ⑨ una fuente llamada `%s` sigue dando nombres validos" % SUCIA)

    return fallos


def comprobar():
    fallos = comprobar_plantillas()

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
        if f.name in PLANTILLAS or f.name[0] == "9" or f.name == POR_FUENTE:
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
        # ⛔ SOLO el bloque `resources:`, y esto lo destapó el
        #   `configMapGenerator` que se añadió el 2026-09-10: sus `files:` son
        #   lineas `  - 11-el-inquilino.yaml`, identicas en forma a las de
        #   `resources:`. Leyendo el fichero entero, esas plantillas contaban
        #   como plataforma y esta misma comprobacion gritaba —con razon en la
        #   forma y sin ella en el hecho— que tenian dos duenos.
        #
        # ⇒ Lo que decide de quien es un fichero no es que aparezca en
        #   `kustomization.yaml`: es de QUE LISTA cuelga. Estar en un generador
        #   es ser un DATO que se monta, no un objeto que se aplica.
        texto = kfile.read_text(encoding="utf-8")
        bloque, dentro = "", False
        for l in texto.splitlines():
            if re.match(r"^resources:\s*$", l):
                dentro = True
                continue
            # Otra clave de primer nivel cierra el bloque. Los comentarios a
            # ras de margen no: `kustomization.yaml` esta lleno de ellos.
            if dentro and l.strip() and not l[0].isspace() and not l.startswith("#"):
                break
            if dentro:
                bloque += l + "\n"
        listados = set(re.findall(r"^\s*-\s+(\S+\.yaml)\s*$", bloque, re.M))
        for f in sorted(MALLA.glob("*.yaml")):
            if f.name == "kustomization.yaml":
                continue
            plantilla, plataforma, prueba = (
                f.name in PLANTILLAS or f.name == POR_FUENTE,
                f.name in listados,
                f.name[0] == "9",
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

    # ── ⑧ Y QUE EL PUESTO DEL APROVISIONADOR LLEVE LO QUE RINDE ───────────
    #
    # ⭐ El `configMapGenerator` de `kustomization.yaml` es lo que el Job monta
    #   en `/guion`, y `gen-inquilino.py` lee las plantillas de su propio
    #   directorio. Una plantilla nueva que se olvide alli no da un error de
    #   configuracion: da un `FileNotFoundError` dentro de un pod, a mitad de un
    #   alta, y sin nadie delante.
    #
    # ⚠️ Al escribir esa lista dejé dicho «al menos es ruidoso». Ruidoso no es
    #   suficiente cuando el ruido lo hace un Job a las tres de la manana: se
    #   comprueba aqui, que es donde hay alguien mirando.
    if kfile.exists():
        gen = ""
        dentro = False
        for l in texto.splitlines():
            if re.match(r"^configMapGenerator:\s*$", l):
                dentro = True
                continue
            if dentro and l.strip() and not l[0].isspace() and not l.startswith("#"):
                break
            if dentro:
                gen += l + "\n"
        montados = set(re.findall(r"^\s*-\s+(\S+\.(?:yaml|py|sh))\s*$", gen, re.M))
        debidos = set(PLANTILLAS) | {POR_FUENTE, "gen-inquilino.py",
                                     "aprovisionar-inquilino.sh",
                                     "converger-inquilinos.sh"}
        for n in sorted(debidos - montados):
            fallos.append(
                "`%s` NO esta en el `configMapGenerator`: el Job no lo tendria "
                "en `/guion` y fallaria a mitad de un alta" % n)
        for n in sorted(montados - debidos):
            fallos.append(
                "el `configMapGenerator` monta `%s`, que no es ni plantilla ni "
                "guion: o sobra, o falta en `PLANTILLAS`" % n)
    print("  ⭐ ⑧ el puesto del aprovisionador lleva las %d plantillas y los 3 guiones"
          % (len(PLANTILLAS) + 1))

    return veredicto(fallos)


def veredicto(fallos):
    if fallos:
        print("\n⛔ EL RENDERIZADOR MIENTE:")
        for x in fallos:
            print("   · " + x)
        return 1
    print("\n✓ la plantilla y el renderizador dicen lo mismo")
    return 0


# ══════════════════════════════════════════════════════════════════════════
def main(argv):
    # ⚠️ El mas largo PRIMERO: `"--comprobar" in argv` es una igualdad de
    #   elementos, no un prefijo, pero el orden deja dicho que son dos modos y
    #   no uno con matiz.
    if "--comprobar-plantillas" in argv:
        return veredicto(comprobar_plantillas())
    if "--comprobar" in argv:
        return comprobar()

    # ⛔ Y hay que saltarse el VALOR de cada opción, no sólo la opción. Filtrar
    #   por `--` dejaba el destino de `--a` contado como si fuera el nombre del
    #   inquilino, así que `gen-inquilino.py acme --a /tmp/x` imprimía la ayuda
    #   —dos «nombres»— en vez de escribir nada. Un uso correcto contestado con
    #   la ayuda se lee como «lo he escrito mal», y manda a mirar el nombre.
    CON_VALOR = ("--arbol", "--entrada", "--fuentes", "--a")
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

    # ⚠️ Separadas por coma y no repetidas: quien llama es un guion de shell,
    #   y una lista en una variable es mas facil de pasar bien que un bucle de
    #   banderas. Vacio significa «ninguna pendiente», que es el caso normal.
    fuentes = [f for f in (valor("--fuentes") or "").split(",") if f]
    hecho = render(nombre, valor("--arbol"), valor("--entrada"), fuentes)
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
