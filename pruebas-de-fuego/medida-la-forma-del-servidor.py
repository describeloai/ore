# -*- coding: utf-8 -*-
"""Que forma tiene el servidor que hay debajo de una consola, y cuanta ya esta.

La pregunta del dia no es «como se pinta la pantalla» sino **que pasa cuando
alguien pulsa un boton**. Las cuatro plataformas que se citan como referencia
contestan esa pregunta de la misma manera, y esa manera se puede copiar.

  A. LAS CUATRO REFERENCIAS     que comparten, y que compra cada rasgo
  B. LOS DOS PLANOS             lo que ya tenemos partido en dos imagenes
  C. LOS GUARDARRAILES          uno a uno, contra el cluster de verdad
  D. LAS TRES FORMAS DEL BOTON  y cual se DERIVA de lo que ya hay
  E. `add source` -> catalogo   paso a paso, y donde se rompe hoy

No se decide nada aqui. Se mide para poder decidirlo.
"""
import json
import pathlib
import re
import subprocess
import textwrap

RAIZ = pathlib.Path(r"C:\ORE")


def parrafo(t, sangria="     ", ancho=72):
    for l in textwrap.wrap(t, ancho):
        print("%s%s" % (sangria, l))


def texto(p):
    try:
        return p.read_text(encoding="utf-8", errors="replace")
    except OSError:
        return ""


def kubectl(*args):
    """Lo que el cluster contesta, o None si no contesta."""
    try:
        r = subprocess.run(
            ["kubectl"] + list(args),
            capture_output=True, text=True, timeout=60,
        )
    except (OSError, subprocess.TimeoutExpired):
        return None
    return r.stdout if r.returncode == 0 else None


def cuenta(*args):
    s = kubectl(*args)
    if s is None:
        return None
    try:
        return len(json.loads(s).get("items", []))
    except ValueError:
        return None


print("== la forma del servidor, medida ==")

# -- A -----------------------------------------------------------------------
print()
print("A - LAS CUATRO REFERENCIAS, Y QUE COMPARTEN")
print()
RASGOS = [
    ("la consola NO es privilegiada",
     "AWS",
     "cada accion de la consola es una llamada a la MISMA API publica, y sale "
     "en CloudTrail igual que una del CLI"),
    ("control plane / data plane",
     "Databricks",
     "la web, el planificador y la API REST viven en el control plane; el "
     "computo que toca el dato del cliente, en el data plane"),
    ("la escritura pasa por un verbo declarado",
     "Foundry",
     "un object type solo admite ediciones POR ACTIONS, y cada Action lleva "
     "sus submission criteria"),
    ("la credencial se PRESTA, no se entrega",
     "Databricks",
     "credential vending: Unity Catalog cambia un OAuth por un token STS "
     "corto contra el objeto concreto"),
    ("el boton DECLARA, y un reconciliador cumple",
     "Kubernetes",
     "se escribe un objeto, admission control lo admite o no, y el "
     "controlador lo lleva al estado declarado"),
]
print("   rasgo                                  de")
print("   " + "-" * 74)
for rasgo, de, _ in RASGOS:
    print("   %-38s %s" % (rasgo, de))
print()
for rasgo, _, porque in RASGOS:
    print("   - %s" % rasgo)
    parrafo(porque, "       ")
print()
parrafo("Los cinco son la MISMA figura vista cinco veces: hay UN sitio donde "
        "se decide, y la pantalla no es ese sitio. Es exactamente la frase "
        "que ORE ya se aplica a si mismo — DESIGN 3.8, la politica se aplica "
        "en un punto unico, nunca el consumidor.")

# -- B -----------------------------------------------------------------------
print()
print("B - LOS DOS PLANOS: ya estan, y son dos imagenes")
print()
dockerfile = texto(RAIZ / "Dockerfile")
etapas = re.findall(r"^FROM\s+(\S+)(?:\s+AS\s+(\S+))?", dockerfile, re.M)
print("   etapa            base")
print("   " + "-" * 74)
for base, nombre in etapas:
    if nombre:
        print("   %-16s %s" % (nombre, base))
print()
print("   la imagen `ore` sale de `scratch`: %s"
      % ("SI" if "scratch" in dockerfile else "no"))
parrafo("Sin certificados y sin cliente TLS. No es que no salga: es que no "
        "sabe. Eso es el control plane, y ya no puede tocar un dato aunque el "
        "codigo tuviera un fallo.")
parrafo("`ore-drivers` es el data plane, y su unico trabajo es leer el "
        "origen. Databricks parte lo mismo por la misma razon y le cuesta dos "
        "cuentas de nube; a nosotros nos cuesta dos FROM.")

# -- C -----------------------------------------------------------------------
print()
print("C - LOS GUARDARRAILES, CONTRA EL CLUSTER DE VERDAD")
print()
vivo = kubectl("version", "--output=json") is not None
if not vivo:
    print("   (sin cluster: esta seccion mide contra los manifiestos)")
    print()

malla = RAIZ / "malla"
manifiestos = "".join(texto(p) for p in sorted(malla.glob("*.yaml")))

sa = cuenta("get", "sa", "-n", "t-demo", "-o", "json")
np = cuenta("get", "networkpolicy", "-n", "t-demo", "-o", "json")
lq = cuenta("get", "localqueue", "-n", "t-demo", "-o", "json")
roles = cuenta("get", "rolebinding", "-n", "t-demo", "-o", "json")
pvc = cuenta("get", "pvc", "-n", "t-demo", "-o", "json")

GUARDAS = [
    ("identidad sin secreto", "Workload Identity",
     bool(sa and sa >= 2),
     "`iam.gke.io/gcp-service-account` en la SA `driver`"),
    ("cuota y equidad", "Kueue",
     bool(lq and lq >= 1),
     "ClusterQueue `cq-demo` en el cohort `ore`"),
    ("por donde se sale", "NetworkPolicy",
     bool(np and np >= 1),
     "deny-all mas la excepcion por `ore.dev/rol: driver`"),
    ("aislamiento del inquilino", "namespace por tenant",
     "t-demo" in manifiestos,
     "`t-demo`, con `ore.dev/tenant` en todo lo que cuelga"),
    ("QUIEN puede hacer QUE", "RBAC por inquilino",
     bool(roles),
     "un Role que diga que la consola crea Jobs y NADA mas"),
    ("donde vive el arbol", "persistencia",
     bool(pvc),
     "hoy `emptyDir`: el repositorio muere con el pod"),
    ("el registro de lo ocurrido", "auditoria",
     "audit" in manifiestos.lower(),
     "el CloudTrail nuestro: quien pulso que, y con que resultado"),
]
print("   guardarrail                que es                     estado")
print("   " + "-" * 74)
for que, con, esta, _ in GUARDAS:
    print("   %-26s %-26s %s" % (que, con, "SI" if esta else "NO"))
hechos = sum(1 for _, _, e, _ in GUARDAS if e)
print()
print("   %d de %d" % (hechos, len(GUARDAS)))
print()
for que, _, esta, nota in GUARDAS:
    if not esta:
        print("   FALTA  %s" % que)
        parrafo(nota, "          ")
print()
parrafo("Los que estan, estan CENTRALIZADOS: no los aplica quien pide, los "
        "aplica el cluster antes de que el pod arranque. Es la propiedad que "
        "se buscaba, y llego antes que la consola.")

# -- D -----------------------------------------------------------------------
print()
print("D - LAS TRES FORMAS DEL BOTON")
print()
FORMAS = [
    ("el servidor ejecuta `ore` en su propio proceso",
     "un binario que lanza `ore` como subproceso y devuelve su salida",
     ["la mas corta de escribir"],
     ["el servidor necesita la credencial del origen, y deja de ser control "
      "plane",
      "una fuente lenta bloquea al servidor",
      "no hay cuota: mil botones son mil procesos"]),
    ("el servidor crea un Job y espera",
     "traduce el boton a un `batch/v1 Job` y sondea hasta que termina",
     ["el computo va donde debe, con la identidad que debe",
      "Kueue ya le pone techo",
      "es LITERALMENTE lo que corre hoy en `94-flujo-completo.yaml`"],
     ["la espera la sostiene el servidor: si se reinicia, se pierde",
      "el resultado hay que sacarlo de los logs, que son prosa"]),
    ("el boton ESCRIBE un objeto y un reconciliador lo cumple",
     "`add source` crea un recurso `Source`; un controlador ve que no tiene "
     "catalogo y lanza el Job",
     ["el discover que 'se dispara solo' NO es el boton llamando dos veces: "
      "es el controlador viendo un origen sin catalogo",
      "sobrevive al reinicio del servidor, porque el deseo esta escrito",
      "reintentar es gratis y ya esta resuelto",
      "es la misma figura que `ore drift-detect`: declarado contra real"],
     ["hay que escribir el controlador",
      "hay que decidir donde vive el arbol ANTES, no despues"]),
]
for i, (nombre, como, pro, con) in enumerate(FORMAS, 1):
    print("   %d - %s" % (i, nombre))
    parrafo(como, "       ")
    for p in pro:
        parrafo("+ %s" % p, "       ")
    for c in con:
        parrafo("- %s" % c, "       ")
    print()
parrafo("La 2 es la que ya corre y no hay que inventarla. La 3 es la que el "
        "flujo pedido DESCRIBE sin nombrarla: «al dar add se redirige al "
        "catalogo donde se muestra la nueva conexion con sus contenidos "
        "descubiertos» es un reconciliador, no un boton que llama a dos "
        "cosas. Y la 1 rompe el reparto que costo todo el dia construir.")

# -- E -----------------------------------------------------------------------
print()
print("E - `add source` -> catalogo, PASO A PASO")
print()
PASOS = [
    ("la consola manda el alta", "HTTP", "FALTA",
     "no hay nada que escuche: `ore serve` esta declarado y no hace nada"),
    ("se guarda la fuente", "`ore source add`", "HECHO",
     "escribe el manifiesto y manda el secreto a `.env.local`"),
    ("...pero el secreto", "`.env.local`", "ROTO",
     "un fichero en el disco del pod no es un Secret de Kubernetes"),
    ("se comprueba que responde", "`ore source check`", "HECHO",
     "verbo suyo, que falla por separado"),
    ("se lee el catalogo", "`ore source catalog`", "HECHO",
     "emite JSON, que es lo que una pantalla necesita"),
    ("se induce", "`ore discover`", "HECHO",
     "emite JSON con `options` y `because` por decision"),
    ("la pantalla lo pinta", "leer el arbol", "FALTA",
     "el arbol vive en un `emptyDir` que muere con el pod"),
]
print("   paso                          con que                 estado")
print("   " + "-" * 74)
for paso, con, estado, _ in PASOS:
    print("   %-29s %-23s %s" % (paso, con, estado))
print()
for paso, _, estado, nota in PASOS:
    if estado != "HECHO":
        print("   %-6s %s" % (estado, paso))
        parrafo(nota, "          ")
print()
falta = sum(1 for _, _, e, _ in PASOS if e != "HECHO")
parrafo("De siete pasos, %d fallan, y ninguno de los %d es logica de "
        "ontologia: son un puerto que escuche, un sitio donde guardar el "
        "secreto y un sitio donde viva el arbol. El sustrato contesta; lo que "
        "falta es quien le pregunta y donde deja la respuesta."
        % (falta, falta))
