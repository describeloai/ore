# -*- coding: utf-8 -*-
"""El plano de identidad que YA existe, y que hay que traer.

No se disena una consola con usuarios encima de un IdP imaginario cuando hay
uno construido, medido y con premisas cerradas viviendo en otro cluster. Esto
lo mira antes de proponer nada.

  A. DONDE VIVE HOY       el otro cluster, y si contesta
  B. EL REALM             lo que declara, leido del fichero
  C. LA AUTORIZACION      que NO esta en Keycloak, y por que importa
  D. CONSUMIR vs OPERAR   las dos cosas que se confunden al decir «migrar»
  E. LOS TRES ALMACENES   git, Artifact Registry y Cloud SQL, cada uno lo suyo

Lee el arbol de Rubix en `C:\\Rubix` sin escribir nada. Si no esta, lo dice y
sigue.
"""
import json
import pathlib
import re
import subprocess
import textwrap
import urllib.error
import urllib.request

RUBIX = pathlib.Path(r"C:\Rubix")
DATAPLANE = pathlib.Path(r"C:\storelyAI")


def parrafo(t, sangria="     ", ancho=72):
    for l in textwrap.wrap(t, ancho):
        print("%s%s" % (sangria, l))


def texto(p):
    try:
        return p.read_text(encoding="utf-8", errors="replace")
    except OSError:
        return ""


def sondear(url):
    """El codigo HTTP, o el motivo de que no haya ninguno."""
    try:
        r = urllib.request.urlopen(url, timeout=15)
        return str(r.status)
    except urllib.error.HTTPError as e:
        return str(e.code)
    except Exception as e:
        return type(e).__name__


print("== el plano de identidad, medido ==")

if not RUBIX.is_dir():
    print()
    print("   (no esta `C:\\Rubix`: esta medida no puede correr)")
    raise SystemExit(0)

# -- A -----------------------------------------------------------------------
print()
print("A - DONDE VIVE HOY")
print()
suspender = texto(DATAPLANE / "infra" / "gke" / "rubix-suspend.sh")
def var(n, txt=None):
    m = re.search(r'^%s="\$\{%s:-([^}]*)\}"' % (n, n), txt or suspender, re.M)
    return m.group(1) if m else "?"

print("   proyecto   %s" % var("PROYECTO"))
print("   cluster    %s" % var("CLUSTER"))
print("   region     %s" % var("REGION"))
print("   pool       %s" % var("POOL"))
print("   namespace  %s" % var("NS"))
print()
bases = re.search(r"^BASES=\(([^)]*)\)", suspender, re.M)
print("   Cloud SQL  %s" % (bases.group(1) if bases else "?"))
costes = re.findall(r"^#\s+(\S+ .*?)\.+ ~?([\d,]+ .*)$", suspender, re.M)
for que, cuanto in costes:
    print("   %-40s %s" % (que, cuanto))
print()
ctx = subprocess.run(["kubectl", "config", "get-contexts", "-o", "name"],
                     capture_output=True, text=True)
otros = [c for c in ctx.stdout.split() if var("CLUSTER") in c]
print("   contexto de kubectl presente:  %s" % ("SI" if otros else "no"))
print()
print("   sondeo, ahora mismo:")
for url, que in [
    ("https://login.paladio.io/realms/rubix/.well-known/openid-configuration",
     "el emisor"),
    ("https://app.paladio.io", "la consola"),
]:
    print("     %-12s %-58s %s" % (que, url[:58], sondear(url)))
print()
parrafo("Un 503 en el emisor no es una averia: es `rubix-suspend.sh off` "
        "haciendo su trabajo — el pool a cero y las dos bases en "
        "`activation-policy NEVER`. El nombre y la entrada sobreviven; lo que "
        "esta parado es lo que cuesta dinero.")

# -- B -----------------------------------------------------------------------
print()
print("B - EL REALM, LEIDO DEL FICHERO")
print()
plantilla = RUBIX / "deploy" / "identidad" / "salida" / "rubix.json"
try:
    d = json.loads(texto(plantilla))
except ValueError:
    d = {}
if d:
    print("   realm                  %s" % d.get("realm"))
    print("   organizations          %s  (Keycloak 26)"
          % ("ACTIVADAS" if d.get("organizationsEnabled") else "no"))
    print("   politica de clave      %s" % d.get("passwordPolicy"))
    print("   segundo factor         %s / %s"
          % (d.get("otpPolicyType"), d.get("otpPolicyAlgorithm")))
    print("   passkeys (WebAuthn)    RP `%s`"
          % (d.get("webAuthnPolicyRpEntityName") or "").strip())
    print("   fuerza bruta           %s, factor %s"
          % (d.get("bruteForceProtected"), d.get("failureFactor")))
    print("   vida del token         %ss   sesion ociosa %ss"
          % (d.get("accessTokenLifespan"), d.get("ssoSessionIdleTimeout")))
    print("   flujos propios         %s"
          % ", ".join(f.get("alias", "") for f in d.get("authenticationFlows", [])))
    print()
    print("   cliente                 publico  navegador  cuenta de servicio")
    print("   " + "-" * 68)
    for c in d.get("clients", []):
        print("   %-23s %-8s %-10s %s"
              % (c.get("clientId"),
                 "si" if c.get("publicClient") else "no",
                 "si" if c.get("standardFlowEnabled") else "no",
                 "si" if c.get("serviceAccountsEnabled") else "no"))
print()
realms = texto(RUBIX / "deploy" / "base" / "identidad" / "realms.yaml")
aal = "AAL2" if "AAL2" in realms else "?"
parrafo("El fichero no lo escribe una persona: lo emite "
        "`deploy/identidad/realm.mjs t_<ULID>`, un realm POR INQUILINO. Y "
        "declara %s contra NIST SP 800-63B-4 — el segundo factor es "
        "REQUIRED, no condicional." % aal)
parrafo("Lo que un `KeycloakRealmImport` NO hace, y esta medido en "
        "`realms.yaml`: mantener. Se salta un realm que ya existe y se "
        "declara `Done: True` igualmente. Quien lo mantiene es "
        "`aplicar-entrada.mjs` contra la Admin API.")

# -- C -----------------------------------------------------------------------
print()
print("C - LA AUTORIZACION NO ESTA EN KEYCLOAK")
print()
canon = texto(RUBIX / "docs" / "canon" / "63-el-plano-de-identidad.md")
print("   roles de realm en la plantilla:   %d"
      % len(d.get("roles", {}).get("realm", [])))
print("   grupos en la plantilla:           %d" % len(d.get("groups", [])))
print()
for clave, que in [
    ("AuthZEN", "la boca de la autorizacion es un estandar de la OpenID Foundation"),
    ("concesion", "el grant vive en la base: sujeto - ambito - papel - estado"),
    ("RFC 8693", "la delegacion tiene forma de `sub` + `act`"),
    ("persona", "el sujeto tiene tipo cerrado: `persona` o `agente`"),
]:
    print("   %-10s %s  %s" % (clave, "SI" if clave in canon else "??", que))
print()
parrafo("Cero roles y cero grupos en el realm NO es un olvido: es la "
        "frontera dibujada a proposito. Keycloak dice QUIEN ERES; quien dice "
        "QUE PUEDES es `rubix.concesion`, y se pregunta por AuthZEN. Es "
        "literalmente `DESIGN` 3.8 de ORE — la politica se aplica en un "
        "punto unico, nunca el consumidor — construida ya, por otra casa de "
        "la misma cabeza.")

# -- D -----------------------------------------------------------------------
print()
print("D - CONSUMIR NO ES OPERAR")
print()
CONSUMIR = [
    ("la URL del emisor", "`https://login.paladio.io/realms/rubix`"),
    ("el JWKS", "se baja del `.well-known`, y se cachea"),
    ("una audiencia", "un cliente nuevo, `ore-serve`, en el realm"),
    ("validar la firma", "y NADA mas: el que valida no decide"),
]
OPERAR = [
    ("la instancia de Cloud SQL", "`rubix-idp`, g1-small, con su ventana de backup"),
    ("el `keycloak-operator`", "y el CR `Keycloak/rubix-idp`"),
    ("la entrada publica", "el IdP es la unica superficie que VE una contrasena"),
    ("un certificado de verdad", "para `login.paladio.io`"),
    ("el nombre, tambien por dentro", "o la guarda anti-SSRF del JWKS no pasa"),
    ("el reconciliador del realm", "`aplicar-entrada.mjs`, porque el import no mantiene"),
]
print("   CONSUMIR el plano de identidad")
for que, con in CONSUMIR:
    print("     %-28s %s" % (que, con))
print()
print("   OPERAR la instancia")
for que, con in OPERAR:
    print("     %-28s %s" % (que, con))
print()
parrafo("Son dos trabajos, y «migrar» los junta sin querer. Consumir se hace "
        "hoy y no necesita que Keycloak viva en `ore-mesh`: un emisor es una "
        "URL. Operar es una mudanza con datos dentro, y solo hace falta si "
        "el cluster viejo se apaga de verdad.")

# -- E -----------------------------------------------------------------------
print()
print("E - LOS TRES ALMACENES, Y NO COMPITEN")
print()
ALMACENES = [
    ("el arbol ontologico", "git",
     "mutable, con historia y con autor: el commit ES la auditoria",
     "no lo tenemos"),
    ("el artefacto sellado", "Artifact Registry",
     "inmutable y versionado, nombrado por digest — lo que `ore pack` emite",
     "YA lo tenemos: ahi viven las dos imagenes"),
    ("el estado de la plataforma", "Cloud SQL",
     "usuarios, organizaciones, concesiones, fuentes, jobs y su traza",
     "YA hay dos instancias, y una ES la de Keycloak"),
]
print("   que                        donde               estado")
print("   " + "-" * 74)
for que, donde, _, estado in ALMACENES:
    print("   %-26s %-19s %s" % (que, donde, estado))
print()
for que, donde, porque, _ in ALMACENES:
    print("   %s -> %s" % (que, donde))
    parrafo(porque, "       ")
print()
parrafo("La pregunta era si Artifact Registry servia de almacen. Sirve, pero "
        "no para esto: guarda cosas INMUTABLES nombradas por su contenido, y "
        "un arbol ontologico se edita. Donde SI encaja exacto es en el otro "
        "extremo del ciclo — el Ontology Bundle firmado que sale de `ore "
        "pack` es, palabra por palabra, un artefacto inmutable versionado.")
parrafo("Y Cloud SQL no hay que «pillarla»: hay dos encendiendose y "
        "apagandose con un script, a 23,92 EUR/mes cada una. La sinergia no "
        "es teorica — la del IdP es literalmente la base de Keycloak.")
