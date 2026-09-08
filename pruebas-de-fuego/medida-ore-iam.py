# -*- coding: utf-8 -*-
"""Que tiene que ser `ore-iam`, medido antes de escribirlo.

Las ocho tablas de `iam` existen y estan vacias. Falta quien las escriba, y
antes de escribirlo hay que contestar cuatro cosas que no son obvias.

  A. LOS VERBOS Y SU ORDEN    cual desbloquea a cual
  B. LAS DOS CARAS            fundar es de OPERADOR; invitar es de PRODUCTO
  C. COMO HABLA CON POSTGRES  la dependencia, contada
  D. LO QUE TODO VERBO ESCRIBE
"""
import pathlib
import subprocess
import textwrap

RAIZ = pathlib.Path(r"C:\ORE")


def parrafo(t, sangria="     ", ancho=72):
    for l in textwrap.wrap(t, ancho):
        print("%s%s" % (sangria, l))


def cierre(crate):
    try:
        s = subprocess.run(
            ["cargo", "tree", "-p", crate, "--edges", "normal", "--prefix", "none"],
            cwd=RAIZ, capture_output=True, text=True, timeout=300).stdout
    except Exception:
        return set()
    return {l.split(" v")[0].strip() for l in s.splitlines() if l.strip()}


print("== `ore-iam`, medido antes de escribirlo ==")

# -- A -----------------------------------------------------------------------
print()
print("A - LOS VERBOS Y SU ORDEN")
print()
VERBOS = [
    ("fundar", "organizacion + persona + pertenencia(dueno) + huella",
     "NADA. Es el arranque en frio", "todo lo demas"),
    ("invitar", "invitacion + huella", "una organizacion con dueno", "admitir"),
    ("admitir", "persona + pertenencia + invitacion.redimida + huella", "una invitacion", "que haya mas de uno"),
    ("conceder", "concesion + huella", "una persona y un recurso", "decidir"),
    ("revocar", "concesion.revocada + huella", "una concesion", "—"),
    ("mirar", "lee, y ESCRIBE huella", "cualquier cosa", "—"),
]
print("   verbo      necesita antes                que desbloquea")
print("   " + "-" * 74)
for v, _, antes, desp in VERBOS:
    print("   %-10s %-29s %s" % (v, antes, desp))
print()
parrafo("==> El primero es `fundar`, y no por gusto: hasta que exista una "
        "organizacion con dueño no hay nadie que pueda invitar a nadie, y las "
        "otras siete tablas no tienen de donde colgar.")

# -- B -----------------------------------------------------------------------
print()
print("B - LAS DOS CARAS, Y POR QUE NO SON LA MISMA")
print()
parrafo("`fundar` tiene un problema que los demas no tienen: **no hay nadie "
        "que lo autorice.** La primera organizacion se crea cuando aun no "
        "existe ninguna persona con potestad, asi que no puede ser una peticion "
        "HTTP autenticada — no habria con que autenticarla.")
print()
print("   cara        quien la usa       como se invoca        que verbos")
print("   " + "-" * 74)
print("   CLI         un operador        un Job del cluster    fundar")
print("   HTTP        la consola         con un token del      invitar, admitir,")
print("                                  realm                 conceder, mirar")
print()
parrafo("Es la misma figura que `ore init`: fundar es un acto de OPERADOR, con "
        "las credenciales del cluster, y deja su huella igual que los demas. "
        "Invitar es un acto de PRODUCTO, y lo hace una persona.")
parrafo("OJO: Y de ahi una regla: la cara CLI **no debe** crecer. El dia que "
        "`ore-iam conceder` exista por linea de ordenes, existira una forma de "
        "conceder que no pasa por la identidad de nadie.")

# -- C -----------------------------------------------------------------------
print()
print("C - COMO HABLA CON POSTGRES")
print()
sin = cierre("ore-entrada")
con = cierre("ore-iam")
if sin and con:
    anade = con - sin
    print("   cierre de `ore-entrada`   %3d crates" % len(sin))
    print("   cierre de `ore-iam`       %3d crates" % len(con))
    print("   lo que añade el cliente   %3d" % len(anade))
    print()
    vetadas = sorted(c for c in anade if any(
        c.startswith(v) for v in
        ["tokio", "openssl", "native-tls", "schannel", "security-framework", "ring", "aws-lc"]))
    print("   de esas, en la lista de vetadas de `ore-cli`: %s"
          % (", ".join(vetadas) if vetadas else "ninguna"))
print()
parrafo("`tokio` entra, y hay que decirlo en vez de colarlo. El veto de "
        "`ore-cli/tests/dependencias.rs` lo justifica asi: «un planificador "
        "asincrono solo hace falta para hablar con algo». Aqui **se habla con "
        "algo** — es un servidor con una base de datos —, asi que es "
        "exactamente el caso que el veto recorta, no una excepcion a el.")
parrafo("Lo que NO entra: ni una crate de FFI. `postgres` sin "
        "`default-features` no lleva TLS, y la conexion es dentro del cluster, "
        "de pod a pod. Es la misma eleccion que ya hace Keycloak con esa misma "
        "base — si un dia deja de bastar, deja de bastar para los dos.")

# -- D -----------------------------------------------------------------------
print()
print("D - LO QUE TODO VERBO ESCRIBE")
print()
parrafo("Cada verbo que cambia algo escribe **dos cosas en la misma "
        "transaccion**: el hecho y su huella. No una despues de la otra — "
        "dentro. Si se pudieran separar, existiria un estado en el que el "
        "hecho ocurrio y nadie lo anoto, y ese estado es indistinguible de un "
        "borrado del registro.")
print()
print("   begin")
print("     insert into iam.<lo que sea>")
print("     insert into iam.huella (quien, agente, operacion, sobre, detalle)")
print("   commit")
print()
parrafo("Y la huella lleva `quien` y `agente` por separado —`sub` y `act` de "
        "RFC 8693—, que es la misma forma que `ore-serve` ya escribe en el "
        "autor y el committer de cada commit de la forja.")
print()
parrafo("* Incluido MIRAR. Es la idea de su `022`: leer lo que hicieron los "
        "demas es la potestad mas barata del catalogo y tambien la mas intima. "
        "Un verbo de lectura que no deja huella es un agujero con forma de "
        "optimizacion.")
