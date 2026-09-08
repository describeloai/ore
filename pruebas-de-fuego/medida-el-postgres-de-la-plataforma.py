# -*- coding: utf-8 -*-
"""Que hay en el Postgres de la plataforma, y si vale la pena uno nuestro.

La propuesta es «`modelo/` es legacy, hagamos un Postgres limpio». Antes de
contestar hay que saber tres cosas: que hay dentro, quien lo usa, y que
significa exactamente «limpio» — porque son tres cosas distintas y sólo una es
barata.

  A. QUE HAY DENTRO       veinte tablas, siete familias
  B. QUE TOCA LA PAGINA   de las siete, cuantas hacen falta
  C. ES LEGACY?           medido por fechas y por quien lo importa
  D. TRES LECTURAS        de «un Postgres limpio»
  E. LO QUE ORE TIENE     y por que eso decide

Lee `C:\\Rubix` sin escribir nada.
"""
import datetime
import pathlib
import re
import subprocess
import textwrap

RUBIX = pathlib.Path(r"C:\Rubix")
MIG = RUBIX / "modelo" / "migraciones"


def parrafo(t, sangria="     ", ancho=72):
    for l in textwrap.wrap(t, ancho):
        print("%s%s" % (sangria, l))


def texto(p):
    try:
        return p.read_text(encoding="utf-8", errors="replace")
    except OSError:
        return ""


print("== el Postgres de la plataforma, medido ==")

if not MIG.is_dir():
    print("   (no estan las migraciones: esta medida no puede correr)")
    raise SystemExit(0)

ficheros = sorted(MIG.glob("*.sql"))
todo = "".join(texto(f) for f in ficheros)
tablas = sorted(set(
    t.lower() for t in re.findall(r"create table(?: if not exists)? ([a-z_.\"]+)", todo, re.I)
    if "." in t
))

# -- A -----------------------------------------------------------------------
print()
print("A - QUE HAY DENTRO")
print()
FAMILIAS = [
    ("gobierno", ["concesion", "ambito_de", "rol", "acceso", "acceso_respaldo"],
     "quien puede que, y la huella de cada acto — la boca es AuthZEN"),
    ("personas", ["sujeto", "organizacion", "invitacion"],
     "el sujeto, su organizacion y como entro"),
    ("celdas", ["celda", "celda_marca", "celda_almacen"],
     "el inquilino y su almacen"),
    ("outbox", ["outbox", "outbox_", "outbox_por_defecto", "consumidor", "evento_visto"],
     "la cola de eventos del pipeline de metadatos"),
    ("catalogo", ["entidad", "estado_faceta"],
     "lo cosechado y el estado de sus facetas"),
    ("busqueda", ["activo_texto", "activo_vector"],
     "texto y embeddings — `pg_trgm` y `pgvector`"),
]
vistas = set()
for nombre, cortas, que in FAMILIAS:
    de_esta = [t for t in tablas if t.split(".")[-1].strip('"') in cortas]
    vistas.update(de_esta)
    print("   %-10s %d tablas  %s" % (nombre, len(de_esta), que))
    print("              %s" % ", ".join(sorted(de_esta)))
sueltas = [t for t in tablas if t not in vistas]
print()
print("   %d tablas en %d migraciones%s" % (
    len(tablas), len(ficheros),
    ("  ·  sin clasificar: " + ", ".join(sueltas)) if sueltas else ""))

# -- B -----------------------------------------------------------------------
print()
print("B - QUE TOCA `/governance/users`")
print()
parrafo("La pagina llama a `organizationMembers`, y el propio `admin/src/"
        "server.ts` dice de donde sale la pertenencia:")
print()
print('     «la pertenencia sale de `rubix.invitacion`, NO de una Organization')
print('      de Keycloak. Sigue sin haber ni una credencial del IdP aqui»')
print()
parrafo("==> De las seis familias, la pagina toca DOS: personas y gobierno. "
        "Las otras cuatro —celdas, outbox, catalogo, busqueda— son el pipeline "
        "de metadatos, y esa pagina no las mira.")
parrafo("Y eso importa para la propuesta: **las tablas que no se usan no "
        "cuestan nada.** Una tabla vacia no se consulta, no se indexa y no se "
        "mantiene. Lo caro de un esquema no es su tamaño: es quien depende de el.")

# -- C -----------------------------------------------------------------------
print()
print("C - ES LEGACY?")
print()
recientes = ficheros[-7:]
print("   las siete ultimas migraciones:")
for f in recientes:
    t = texto(f)
    tit = re.search(r"^--\s*(\d+ · .+)$", t, re.M)
    print("     %-32s %s" % (f.name, (tit.group(1) if tit else "")[:40]))
print()
fechas = sorted(set(re.findall(r"20\d\d-\d\d-\d\d", todo)))
if fechas:
    print("   fechas que aparecen en el esquema: %s … %s" % (fechas[0], fechas[-1]))
print()
try:
    imp = subprocess.run(
        ["grep", "-rl", "paladio-modelo", str(RUBIX / "admin" / "src"), str(RUBIX / "api" / "src")],
        capture_output=True, text=True, timeout=60).stdout.strip().splitlines()
except Exception:
    imp = []
print("   ficheros de `admin` y `api` que importan `paladio-modelo`: %d" % len(imp))
print()
parrafo("==> **Legacy no, en el sentido de abandonado.** Las siete migraciones "
        "mas nuevas son justo el plano de gobierno —organizacion, potestades, "
        "invitacion y sus huellas— y `admin` y `api` estan escritos contra el. "
        "Es codigo vivo con sus motivos escritos.")
parrafo("Lo que SI es cierto, y es otra frase: **son DOS nucleos.** ORE tiene "
        "el suyo, y este tiene el suyo. Eso es una observacion de producto, no "
        "una de mantenimiento.")

# -- D -----------------------------------------------------------------------
print()
print("D - TRES LECTURAS DE «UN POSTGRES LIMPIO»")
print()
LECTURAS = [
    ("1) limpio de DATOS, su esquema",
     "una base vacia + las 22 migraciones",
     "media hora",
     "`admin` y `api` funcionan sin tocar una linea. Y ya es «nuestro "
     "Postgres en el cluster nuevo»: que el esquema sea suyo no hace que los "
     "datos lo sean"),
    ("2) limpio de TABLAS, su esquema recortado",
     "correr sólo las migraciones de personas y gobierno",
     "no se puede",
     "`migrar.mjs` las corre en orden y en cadena; saltarse una rompe las "
     "siguientes. Y no compraria nada: una tabla vacia no cuesta"),
    ("3) esquema NUESTRO, sin `modelo/`",
     "diseñar persona, organizacion, concesion e invitacion desde cero",
     "un proyecto",
     "no es una decision de Postgres: es tirar `admin` y `api`, que estan "
     "escritos contra `modelo/`. Y hay que reescribir lo que ya funciona"),
]
for n, como, cuanto, nota in LECTURAS:
    print("   %s" % n)
    print("     %-46s coste: %s" % (como, cuanto))
    parrafo(nota, "       ")

# -- E -----------------------------------------------------------------------
print()
print("E - LO QUE ORE YA TIENE, Y POR QUE ESO DECIDE")
print()
SOLAPE = [
    ("una persona", "NO", "`owner:` es una cadena. Nadie la respalda"),
    ("una organizacion", "NO", "no existe el concepto"),
    ("una concesion", "NO", "ORE gobierna el FLUJO, no el acceso"),
    ("una invitacion", "NO", "no existe"),
    ("la huella de un acto", "a medias", "el commit de la forja, desde hoy"),
    ("el gobierno del flujo", "SI", "reticulo, conductos y `OOS4xxx` — y eso `modelo/` no lo tiene"),
]
print("   concepto              en ORE      ")
print("   " + "-" * 74)
for q, hay, nota in SOLAPE:
    print("   %-21s %-11s %s" % (q, hay, nota))
print()
parrafo("Los dos nucleos **no se solapan: se complementan.** `modelo/` sabe "
        "quien es la gente y que puede; ORE sabe que significan los datos y "
        "hasta donde pueden viajar. La frontera ya la acordamos: la concesion "
        "puede NEGAR, no puede conceder por encima del conducto.")
parrafo("==> La 3) no es cara por el SQL. Es cara porque habria que volver a "
        "escribir la mitad que ORE no tiene, y que ahi ya esta escrita, "
        "probada y con sus motivos al lado.")
print()
parrafo("Y una cosa que la 1) compra y no es obvia: mientras `modelo/` viva en "
        "una base que levantamos nosotros con un fichero, retirarlo el dia que "
        "ORE cubra esa mitad cuesta borrar un despliegue. Eso es reversible. "
        "Reescribirlo primero, no.")
