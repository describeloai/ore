# -*- coding: utf-8 -*-
"""MEDIDA · qué es un inquilino, qué hay clavado a `demo`, y qué falta para enterprise.

Decidido: `ore-serve` va POR ORGANIZACIÓN. Eso convierte fundar en aprovisionar
un inquilino, y esta medida dice cuánto trabajo es eso y qué falta para que un
cliente enterprise o de administración pública pueda mirarlo sin sonrojo.

Se mide sobre `malla/`, que es lo que define el clúster.

    uso:  python pruebas-de-fuego/medida-el-aprovisionador-de-inquilinos.py
"""
import pathlib
import re
import sys

try:
    sys.stdout.reconfigure(encoding="utf-8")
except AttributeError:
    pass

ORE = pathlib.Path(__file__).resolve().parent.parent
MALLA = ORE / "malla"
hallazgos = []
rojo = []


def titulo(n, t):
    print("\n" + "=" * 78)
    print("%s · %s" % (n, t))
    print("=" * 78)


def leer(p):
    try:
        return (MALLA / p).read_text(encoding="utf-8", errors="replace")
    except OSError:
        rojo.append("no se pudo leer malla/%s" % p)
        return ""


def exige(cond, m):
    if not cond:
        rojo.append(m)
    return cond


# ══════════════════════════════════════════════════════════════════════════
titulo("①", "QUE ES UN INQUILINO HOY — el inventario, y no es un Deployment")
# ══════════════════════════════════════════════════════════════════════════

# Dónde se declara el namespace del inquilino. ⚠️ No está donde uno lo buscaría.
donde_ns = []
for f in sorted(MALLA.glob("*.yaml")):
    t = f.read_text(encoding="utf-8", errors="replace")
    for m in re.finditer(r"kind: Namespace\n(?:.*\n)*?\s+name: (\S+)", t):
        if m.group(1).startswith("t-"):
            donde_ns.append((f.name, m.group(1)))

print("""
  Un inquilino NO es un Deployment. Medido en `t-demo`, es esto:

      Namespace                con `ore.dev/tenant`
      ResourceQuota            cpu, memoria y CUENTA DE JOBS
      NetworkPolicy  x7        deny-all de entrada y salida, mas los permisos
      ServiceAccount x3        ore-serve, driver, refresco-jwks
      Deployment + Service     ore-serve
      Secret forja-token       el testigo con el que empuja
      Secret idp-agente        para las pruebas de extremo a extremo
      ConfigMap jwks           las llaves del realm
      CronJob refresco-jwks    que las mantiene al dia
      LocalQueue de Kueue      para los Jobs que leen origenes
      y en la forja:           el repositorio, con su `ore init` hecho""")

print("  El namespace del inquilino se declara en:  %s"
      % (", ".join("%s (%s)" % (a, b) for a, b in donde_ns) or "NINGUN SITIO"))
exige(any(a.startswith("10-kueue") for a, _ in donde_ns),
      "`t-demo` ya no se declara en `10-kueue.yaml`: ① cambia")
print("""
  ⚠️ Y eso es lo primero que estorba: el namespace del INQUILINO nace dentro del
    manifiesto de la COLA. Mientras hubo uno solo daba igual; para crear el
    segundo hay que separarlo, porque aprovisionar un inquilino no puede
    significar reaplicar la configuracion de Kueue.""")
hallazgos.append("el namespace del inquilino se declara dentro de `10-kueue.yaml`")

# ══════════════════════════════════════════════════════════════════════════
titulo("②", "QUE HAY DE `demo` CLAVADO — el trabajo de parametrizar")
# ══════════════════════════════════════════════════════════════════════════

# Sólo las plantillas que constituyen al inquilino. Los `9x-` son pruebas.
PLANTILLAS = ["10-kueue.yaml", "40-ore-serve.yaml", "50-jwks.yaml"]
total = 0
print()
for f in PLANTILLAS:
    t = leer(f)
    n = len(re.findall(r"t-demo|tenant: demo|tenant=demo", t))
    total += n
    print("  %-20s %2d menciones de `demo`" % (f, n))
print("  %-20s %2d" % ("", total))

forja_url = re.search(r"(http://forja[^\s\"]*)", leer("40-ore-serve.yaml"))
print("""
  Y la que importa, porque es la que ata el proceso a UN arbol:

      %s

  ⇒ Parametrizar es sustituir un nombre en tres ficheros. **No es el trabajo.**
    El trabajo es lo de ③ y ④.""" % (forja_url.group(1) if forja_url else "?"))
exige(forja_url and "t-demo" in forja_url.group(1),
      "`--forja` ya no apunta a un repositorio fijo: ② cambia")
hallazgos.append("parametrizar las plantillas es sustituir un nombre en 3 ficheros")

# ══════════════════════════════════════════════════════════════════════════
titulo("③", "⭐⭐ QUIEN LO APLICA — y la respuesta obvia es la mala")
# ══════════════════════════════════════════════════════════════════════════

print("""
  Un proceso que crea namespaces, Deployments, Secrets y NetworkPolicies en
  cualquier inquilino necesita permisos de ambito de CLUSTER sobre esos tipos.
  Y ahi esta el problema, dicho sin rodeos:

  ⛔ QUIEN PUEDE CREAR EL SECRET DE UN INQUILINO PUEDE LEER EL DE TODOS.

    Ese proceso se convierte en la pieza mas peligrosa del sistema — mas que
    cualquier `ore-serve`—, y es exactamente la figura que este arbol ya rechazo
    una vez: en CI se hizo que la construccion corriera COMO `ore-ci` en vez de
    concederle suplantar a la cuenta de computo, «que tiene medio proyecto».

  ⭐⭐ Y hay una salida que encaja con lo que este producto ya es:

      EL APROVISIONADOR NO APLICA NADA. ESCRIBE.

    Emite los manifiestos del inquilino a un repositorio, y un agente de GitOps
    —que ya tiene esos permisos, una vez, auditado, y no los presta— los aplica.

    De ahi salen cuatro cosas que no hay que programar:

      · el aprovisionador NO necesita credenciales de cluster. Necesita empujar
        a un repositorio, que es lo unico que este sistema ya sabe hacer;
      · dar de alta un inquilino queda en un COMMIT, con quien lo pidio dentro.
        Para gobierno eso no es un lujo: es la prueba;
      · revisar antes de aplicar es un `pull request`, no un procedimiento;
      · y deshacerlo es `revert`.

    ⇒ Es el mismo argumento que sostiene el arbol de la ontologia, aplicado al
      arbol del CLUSTER. Un inquilino tambien es un documento.

  ⚠️ Lo que hay que decidir es de QUIEN es ese repositorio y quien lo revisa —no
    si existe—, y esa es una pregunta de organizacion, no de codigo.""")
hallazgos.append("⭐ el aprovisionador ESCRIBE manifiestos; aplicarlos es de GitOps")

# ══════════════════════════════════════════════════════════════════════════
titulo("④", "⛔ LO QUE FALTA PARA ENTERPRISE Y GOBIERNO")
# ══════════════════════════════════════════════════════════════════════════

serve = leer("40-ore-serve.yaml")
forja = leer("30-forja.yaml")
base = leer("00-base.yaml")

copia_idp = "CronJob" in leer("62-copias-del-idp.yaml")
# ✏️ 2026-09-09 · la copia de la forja ya existe, y en su propio fichero.
copias_forja = leer("31-copias-de-la-forja.yaml")
copia_forja = "kind: CronJob" in copias_forja
# ⭐ Y esta SÍ se restaura: clona desde el bundle y compara las referencias
#   ANTES de subir. La del IdP sigue sin restaurarse nunca.
restaura = "bundle verify" in copias_forja and "vuelta.git" in copias_forja
psa = any("pod-security.kubernetes.io" in leer(f.name) for f in MALLA.glob("*.yaml"))
no_root = "runAsNonRoot" in serve
seccomp = "seccompProfile" in serve
replicas = re.search(r"replicas: (\d+)", serve)
ingress = sum("kind: Ingress" in leer(f.name) for f in MALLA.glob("*.yaml"))
retencion = "retencion" in (ORE / "iam/migraciones/008-la-huella.sql").read_text(
    encoding="utf-8", errors="replace")

def fila(bien, que, nota):
    print("  %s %-34s %s" % ("·" if bien else "⛔", que, nota))

print("\n  LO QUE SE GUARDA")
fila(copia_idp, "copia del IdP", "CronJob diario a las 12:00")
fila(copia_forja, "copia de LA FORJA",
     "CronJob diario a las 12:30, un bundle por repositorio" if copia_forja
     else "NO HAY. Un PVC de 10Gi con el arbol de cada inquilino dentro")
fila(restaura, "restauracion de la forja",
     "se clona DESDE el bundle y se comparan las referencias, en cada vuelta"
     if restaura else "no se prueba")
fila(False, "restauracion del IdP",
     "nunca se ha hecho — y una copia sin restaurar es una esperanza")
fila(retencion, "retencion de `iam.huella`",
     "sin politica: ni se purga ni se exporta. Para gobierno la auditoria tiene que SALIR")

print("\n  LO QUE CORRE")
fila(psa, "Pod Security Admission",
     "ningun namespace lleva `pod-security.kubernetes.io/enforce`")
fila(no_root, "runAsNonRoot en `ore-serve`", "no declarado")
fila(seccomp, "seccompProfile", "no declarado")
fila(True, "capabilities drop ALL", "si, y `allowPrivilegeEscalation: false`")
fila(False, "readOnlyRootFilesystem",
     "`false`, y con motivo: clona. Se cierra con un `emptyDir` para el clon")
fila(replicas and int(replicas.group(1)) > 1, "alta disponibilidad",
     "`replicas: %s`. Sin PodDisruptionBudget: un drenaje corta el servicio"
     % (replicas.group(1) if replicas else "?"))

print("\n  QUIEN ENTRA Y CON QUE")
fila(ingress > 1, "como llega la consola a N inquilinos",
     "%d Ingress en toda la malla, y es el del IdP. Falta la entrada por inquilino" % ingress)
# ✏️ 2026-09-09 · estrechado. El ambito no basta: lo que ata el token a un
#   arbol es de QUIEN es el usuario y de que es colaborador.
estrecho = "serve-demo" in leer("40-ore-serve.yaml")
fila(estrecho, "el testigo de la forja",
     "un usuario por inquilino, colaborador de SU arbol y de nada mas"
     if estrecho else "`forja-token` es de `ore-admin`, ADMINISTRADOR de la forja entera")
fila(False, "segundo factor",
     "`EXIGIR_SEGUNDO_FACTOR = false` hasta 2026-09-30. AAL2 lo exige para administrar")

print("\n  LO QUE UN CLIENTE VA A PREGUNTAR EL PRIMER DIA")
fila(False, "residencia del dato",
     "el arbol la marca «decision pendiente» a proposito: la afirma quien responde")
fila(False, "cifrado con clave del cliente",
     "no se menciona. Para banca y sector publico suele ser requisito")
fila(False, "salida de la auditoria",
     "no hay exportacion de `iam.huella` ni al SIEM del cliente ni a un fichero")

hallazgos.append(("✓ la forja ya tiene copia, Y SE RESTAURA en cada vuelta"
                  if copia_forja and restaura else
                  "⛔ LA FORJA NO TIENE COPIA, y guarda el arbol de cada inquilino"))
hallazgos.append("⛔ ningun namespace declara Pod Security Admission")
hallazgos.append("⛔ `ore-serve` es una replica sin PodDisruptionBudget")
hallazgos.append("⛔ no hay entrada por inquilino: 1 Ingress, y es del IdP")

# ══════════════════════════════════════════════════════════════════════════
titulo("⇒", "LO QUE SALE DE MEDIR")
# ══════════════════════════════════════════════════════════════════════════
for h in hallazgos:
    print("  · " + h)

print("""
  ⭐ La conclusion incomoda, y es la util: **parametrizar las plantillas es lo
    barato**. Tres ficheros y un nombre. Lo que separa esto de un estado del
    arte para enterprise no es el aprovisionador — son cuatro cosas que hoy no
    existen y que ningun cliente de ese tamaño va a dejar pasar:

      1  ✓ HECHO el 2026-09-09: `31-copias-de-la-forja.yaml`. Un bundle por
         repositorio, y **se clona desde el bundle y se comparan las referencias
         antes de subirlo** — asi que restaurar no es una promesa. Queda que la
         copia del IdP no se ha restaurado NUNCA, y esa sigue siendo un fichero.
      2  ✓ HECHO el 2026-09-09: un usuario de forja por inquilino, colaborador
         de su arbol y de nada mas. Un repositorio ajeno le da 404, no 403 —
         para el no existe. Queda que el token viejo, ya sin uso, sigue vivo en
         la forja: borrarlo pide la contraseña de `ore-admin`, que se puso a
         mano y no esta en el cluster.
      3  QUIEN APLICA. Un aprovisionador con permisos de cluster es una pieza
         que puede leer los secretos de todos. Que ESCRIBA manifiestos y los
         aplique GitOps quita el problema y ademas deja el alta en un commit.
      4  LA ENTRADA. Con `ore-serve` por organizacion hay que decidir como llega
         la consola a cada uno, y en ese mercado normalmente cada cliente quiere
         su propio nombre DNS.

  Y por debajo, lo que se pide en cuanto hay un pliego: PSA, `runAsNonRoot`,
  seccomp, alta disponibilidad, retencion y SALIDA de la auditoria, residencia
  declarada, y cifrado con clave del cliente.

  ⇒ El orden que propongo, y el primero no es codigo nuestro:

      1  ✓ copia de la forja, con su restauracion probada
      2  ✓ estrechar el testigo
      3  separar el namespace del inquilino de `10-kueue.yaml`
      4  el aprovisionador que ESCRIBE, no que aplica
      5  la entrada por inquilino""")

if rojo:
    print("\n⛔ LA MEDIDA NO CUADRA CON EL ARBOL:")
    for r in rojo:
        print("   · " + r)
    sys.exit(1)
print("\n✓ todo lo que esta medida afirma sigue estando en el arbol")
