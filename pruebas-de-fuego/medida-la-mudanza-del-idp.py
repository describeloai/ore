# -*- coding: utf-8 -*-
"""Que clase de mudanza es traer el IdP al cluster nuevo.

`ore-serve` ya verifica tokens; lo que falta para quitar el modo de banco es un
emisor vivo en la cuenta que tiene creditos. Antes de mover nada: que hay que
mover, que ya esta, y que NO puedo hacer yo.

  A. QUE CLASE DE MUDANZA ES   segun el registro de la propia plataforma
  B. LAS SIETE PIEZAS          cual esta en `ore-mesh` y cual no
  C. EL EMISOR                 el nombre no se renombra dos veces
  D. LA BASE                   Cloud SQL contra Postgres en el cluster
  E. LO QUE NO ES MIO          el DNS, y por que

Lee `C:\\Rubix` sin escribir nada.
"""
import json
import pathlib
import re
import subprocess
import textwrap

RUBIX = pathlib.Path(r"C:\Rubix")
PROYECTO = "project-8853a180-450d-47be-b83"


def parrafo(t, sangria="     ", ancho=72):
    for l in textwrap.wrap(t, ancho):
        print("%s%s" % (sangria, l))


def texto(p):
    try:
        return p.read_text(encoding="utf-8", errors="replace")
    except OSError:
        return ""


def correr(*args):
    try:
        r = subprocess.run(list(args), capture_output=True, text=True, timeout=120)
    except (OSError, subprocess.TimeoutExpired):
        return None
    return r.stdout if r.returncode == 0 else None


print("== la mudanza del IdP, medida ==")

if not RUBIX.is_dir():
    print("   (no esta `C:\\Rubix`: esta medida no puede correr)")
    raise SystemExit(0)

realms = texto(RUBIX / "deploy" / "base" / "identidad" / "realms.yaml")
kc = texto(RUBIX / "deploy" / "base" / "identidad" / "keycloak.yaml")

# -- A -----------------------------------------------------------------------
print()
print("A - QUE CLASE DE MUDANZA ES")
print()
censo = re.search(r"usuarios (\d+) . sesiones (\d+) . grupos (\d+) . IdP federados (\d+)", realms)
if censo:
    print("   El censo que la plataforma se hizo a si misma antes de borrar un realm:")
    print("     usuarios %s · sesiones %s · grupos %s · IdP federados %s" % censo.groups())
print()
# El comentario del CR va partido en varias lineas con `#` delante, asi que se
# normaliza antes de buscar: un `in` sobre el texto crudo no lo encuentra y
# haria pasar por «no dice nada» algo que dice bastante.
plano = re.sub(r"\s*#\s*", " ", kc)
cero = re.search(r"(0 usuarios, 0 sesiones, 0\s+tokens emitidos)", plano)
print("   y el CR, al renombrar el emisor el 2026-08-26:")
print("     «el momento mas barato que va a existir: %s»"
      % (re.sub(r"\s+", " ", cero.group(1)) if cero else "?"))
print()
parrafo("==> **Esto NO es una migracion de datos.** No hay usuarios que mover, "
        "no hay sesiones que preservar y no hay tokens vivos que invalidar. La "
        "configuracion no se copia: se GENERA —`deploy/identidad/realm.mjs`—, "
        "asi que lo que se muda es un despliegue declarativo, no una base.")
parrafo("OJO: Con una condicion que hay que comprobar y no puedo: que no se "
        "hayan creado personas desde entonces. Si las hay, entonces si hace "
        "falta un `pg_dump` de `rubix-idp` y esto cambia de clase.")

# -- B -----------------------------------------------------------------------
print()
print("B - LAS SIETE PIEZAS")
print()
apis = correr("gcloud", "services", "list", "--enabled", "--project", PROYECTO,
              "--format=value(config.name)") or ""
crds = correr("kubectl", "get", "crd", "-o", "name") or ""
sc = correr("kubectl", "get", "storageclass", "-o", "name") or ""
zonas = correr("gcloud", "dns", "managed-zones", "list", "--project", PROYECTO,
               "--format=value(name)") or ""
ips = correr("gcloud", "compute", "addresses", "list", "--project", PROYECTO,
             "--format=value(name)") or ""

PIEZAS = [
    ("el operador de Keycloak", "keycloak.org" in crds,
     "los CRD `Keycloak` y `KeycloakRealmImport`"),
    ("la imagen del IdP, cocida", False,
     "`C:\\Rubix\\idp\\Dockerfile` — hay que cocerla aqui: la de alla vive en "
     "el registro de `trino-k8s`"),
    ("una base para Keycloak", "sqladmin.googleapis.com" in apis,
     "Cloud SQL sin habilitar; la alternativa es Postgres en el cluster"),
    ("un disco que no se borre", "retiene" in sc,
     "la clase `retiene`, que ya existe por la forja"),
    ("el realm", True,
     "`realm.mjs` lo genera, y `salida/rubix.json` esta escrito"),
    ("una IP y un certificado", bool(ips.strip()),
     "para la entrada publica"),
    ("el nombre, por dentro", False,
     "que `login.paladio.io` resuelva al Keycloak de AQUI dentro del cluster"),
]
print("   pieza                        esta   que es")
print("   " + "-" * 76)
for que, esta, _ in PIEZAS:
    print("   %-28s %-6s" % (que, "SI" if esta else "NO"))
print()
for que, esta, nota in PIEZAS:
    if not esta:
        print("   FALTA  %s" % que)
        parrafo(nota, "          ")

# -- C -----------------------------------------------------------------------
print()
print("C - EL EMISOR, Y POR QUE NO SE RENOMBRA DOS VECES")
print()
m = re.search(r"^\s*hostname: (https://\S+)", kc, re.M)
print("   hostname (el `iss` de todos los tokens):  %s" % (m.group(1) if m else "?"))
print()
parrafo("El CR lo dice con todas las letras: «no es configuracion, es una "
        "identidad publicada», y «se renombra una vez, y es la ultima». "
        "Levantarlo aqui con un nombre interno para renombrarlo despues seria "
        "hacer justo lo que ellos evitaron.")
parrafo("==> Se levanta con `login.paladio.io` DESDE EL PRIMER DIA, y se hace "
        "que ese nombre resuelva dentro del cluster al Keycloak de aqui. Es la "
        "figura de `deploy/base/identidad/nombre.yaml`: un Service con IP fija "
        "y `hostAliases` en quien lo consume, porque la guarda anti-SSRF del "
        "JWKS exige que el `jwks_uri` cuelgue del MISMO origen que el emisor.")

# -- D -----------------------------------------------------------------------
print()
print("D - LA BASE")
print()
suspender = texto(pathlib.Path(r"C:\storelyAI\infra\gke\rubix-suspend.sh"))
precio = re.search(r"Cloud SQL rubix-idp\s+\(g1-small\) \.+ ~?([\d,]+ .*)", suspender)
print("   lo que cuesta hoy la de alla, medido por ellos contra la Billing API:")
print("     Cloud SQL rubix-idp (g1-small)   %s" % (precio.group(1) if precio else "?"))
print()
OPCIONES = [
    ("Cloud SQL g1-small", "~24 EUR/mes", "copias automaticas, PITR, y nada que operar",
     "es la linea de coste mas cara de todo esto, y los creditos son finitos"),
    ("Cloud SQL db-f1-micro", "~9 EUR/mes", "lo mismo, y sobra: el pico medido de conexiones fue 6",
     "sigue habiendo que habilitar el API, la IP privada y el proxy"),
    ("Postgres en el cluster", "~1 EUR/mes", "el disco de la clase `retiene`, que ya existe",
     "las copias hay que escribirlas: un volcado a un bucket, y sin PITR"),
]
print("   opcion                   coste        ")
print("   " + "-" * 76)
for n, c, _, _ in OPCIONES:
    print("   %-24s %s" % (n, c))
print()
for n, _, pro, con in OPCIONES:
    print("   %s" % n)
    parrafo("+ %s" % pro, "       ")
    parrafo("- %s" % con, "       ")
print()
parrafo("La diferencia entre la primera y la tercera son ~23 EUR/mes sobre un "
        "colchon de 300 dolares, con un cluster que ya consume. Y la tercera "
        "es la unica REVERSIBLE barato: pasar de Postgres en el cluster a "
        "Cloud SQL es un `pg_dump | psql`; al reves tambien, pero habiendo "
        "pagado el año.")

# -- E -----------------------------------------------------------------------
print()
print("E - LO QUE NO ES MIO")
print()
ns = correr("nslookup", "-type=NS", "paladio.io") or ""
servidores = sorted(set(re.findall(r"nameserver\s*=\s*(\S+)", ns)))
print("   quien sirve `paladio.io`:  %s" % (", ".join(servidores) or "?"))
print("   zonas DNS en el proyecto:  %s" % (zonas.strip() or "ninguna"))
print()
parrafo("El dominio no se gestiona en Google Cloud: los servidores de nombres "
        "son del registrador. Apuntar `login.paladio.io` al cluster nuevo es "
        "un cambio de un registro A en ese panel, y lo hace una persona.")
print()
parrafo("Y por eso la mudanza se parte en dos, que es como su propio `F3` ya "
        "estaba partido —maquinas y navegador—:")
print()
print("     1  DENTRO       Keycloak vivo, el realm importado, el nombre")
print("                     resolviendo por dentro, `ore-serve` verificando")
print("                     tokens de VERDAD y el JWKS refrescandose solo.")
print("                     ==> quita el modo de banco para las MAQUINAS")
print()
print("     2  FUERA        una IP, un certificado y el registro DNS.")
print("                     ==> deja entrar a las PERSONAS por el navegador")
print()
parrafo("El 1 se puede hacer entero hoy y no toca nada de la cuenta vieja. El "
        "2 empieza con un cambio en el registrador.")
