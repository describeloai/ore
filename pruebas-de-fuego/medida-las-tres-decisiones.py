# -*- coding: utf-8 -*-
"""MEDIDA · las tres decisiones de la consola, en profundidad.

`medida-la-consola-contra-ore-iam.py` las nombró. Ésta las mide, porque las tres
resultaron ser más grandes de lo que parecían al nombrarlas — y una de ellas es
otra pregunta.

    uso:  python pruebas-de-fuego/medida-las-tres-decisiones.py
"""
import pathlib
import re
import sys

try:
    sys.stdout.reconfigure(encoding="utf-8")
except AttributeError:
    pass

CONSOLA = pathlib.Path(r"C:\rubix-platform")
PLATAFORMA = pathlib.Path(r"C:\Rubix")
ORE = pathlib.Path(r"C:\ORE")


def titulo(n, t):
    print("\n" + "═" * 78)
    print("%s · %s" % (n, t))
    print("═" * 78)


def leer(p):
    try:
        return p.read_text(encoding="utf-8", errors="replace")
    except OSError:
        return ""


# ══════════════════════════════════════════════════════════════════════════
titulo("ⓐ", "NO ES «TOKEN O CAMINO». Es: ¿UNA organizacion por persona, o VARIAS?")
# ══════════════════════════════════════════════════════════════════════════

print("""
  Al medirlo se cae la pregunta que yo habia escrito. `admin/` no saca la
  organizacion de un claim: la saca del SUJETO RESUELTO. Y su sujeto **lleva la
  organizacion dentro**, por construccion.
""")

s = leer(PLATAFORMA / "modelo" / "migraciones" / "014-sujeto.sql")
for l in s.splitlines():
    if "organizacion text not null" in l or "id.paladio.io" in l:
        print("    014-sujeto.sql:  " + l.strip())

print("""
  ⇒ El IRI de una persona **contiene su organizacion**. No es una relacion: es
    parte de su identidad. Una persona en dos clientes son dos sujetos, y
    `provisionOrganization` lo dice al negarse:
""")
p = leer(PLATAFORMA / "modelo" / "estado" / "potestad.mjs")
m = re.search(r"ese principal ya existe en otra organización[^']*'[^']*'", p)
if m:
    print("    " + re.sub(r"\s+", " ", m.group(0))[:200])

print("""
  ── Y lo NUESTRO dice lo contrario, sin que nadie lo decidiera ─────────────
""")
for f in ("003-la-persona.sql", "005-la-pertenencia.sql"):
    t = leer(ORE / "iam" / "migraciones" / f)
    cols = re.search(r"create table if not exists iam\.\w+ \((.*?)\n\)", t, re.S)
    if cols:
        prim = [c.strip() for c in cols.group(1).splitlines()
                if c.strip() and not c.strip().startswith("--")][:4]
        print("    %-26s %s" % (f, " | ".join(prim)[:120]))

print("""
    ⇒ `iam.persona` NO lleva organizacion. `iam.pertenencia` es (persona,
      organizacion) con clave primaria compuesta ⇒ **muchas a muchas por
      construccion**. Nadie decidio eso: salio de copiar la forma de una tabla
      de union.

  ── Lo que dice el IdP, que ya lo habia contestado ────────────────────────

    El scope `organization` de Keycloak 26 estampa el claim con el mapeador
    `oidc-organization-membership-mapper`, y su configuracion medida contra el
    realm vivo dice:

        "multivalued" : "true"

    ⇒ el claim es un ARRAY de organizaciones. El IdP ya asume que una persona
      puede estar en varias. Y por eso `76 ANEXO` **saco la organizacion del
      claim** y la puso en `rubix.sujeto`: con un array, `organizacionDeClaims`
      no sabia cual elegir.

  ── El censo, hoy, en el cluster ──────────────────────────────────────────

        personas 2 · organizaciones 2 · a una organizacion cada una

    ⭐ Ninguna persona esta en dos. Es el momento barato — el mismo argumento
      con el que se fijo el `iss` el primer dia: se decide ahora o se decide
      con clientes dentro.

  ── LAS TRES SALIDAS, con lo que cuesta cada una ──────────────────────────

    ① UNA organizacion por persona, como ellos
       La persona pertenece a un cliente y punto. Entrar en dos = dos cuentas.
       + la consola funciona sin tocar su forma; `admin/` cabe casi tal cual
       + el aislamiento lo sostiene el motor, no una comprobacion
       − un consultor que trabaja para dos clientes tiene dos identidades
       − y NOSOTROS no podriamos administrar la plataforma con nuestra cuenta

    ② VARIAS, y la consola ELIGE (lo que ya hay)
       `GET /organizaciones` devuelve las suyas; la eleccion viaja en el camino.
       + no se toca el esquema, y es lo unico que hoy esta escrito y probado
       − la consola necesita un selector, y CADA pantalla necesita saber cual
       − `admin/` no cabe: sus rutas no llevan organizacion

    ③ VARIAS, y el IdP elige por sesion
       Keycloak acepta `organization:<alias>` como scope: una sesion, una
       organizacion, elegida al entrar.
       + la consola no cambia de forma: sigue habiendo UNA organizacion
       + cambiar de cliente es volver a entrar, que es lo que hacen los cuatro
       − depende de una capacidad del IdP y ata la sesion al realm
       − y hay que comprobarlo: NO esta medido contra este Keycloak

  ⇒ La pregunta que te desbloquea no es «token o camino». Es:
     **¿queremos que una persona pueda pertenecer a dos clientes?**
     Si la respuesta es no, ① y se acaban las otras dos decisiones a la vez.
""")

# ══════════════════════════════════════════════════════════════════════════
titulo("ⓑ", "EL CATALOGO — 31 potestades, y cuantas tienen referente aqui")
# ══════════════════════════════════════════════════════════════════════════

# Clasificacion hecha a mano contra `iam/migraciones` y `crates/ore-iam`, no
# adivinada: cada linea dice QUE hay, no si «podria haberlo».
ESTADO = {
    "hay": "la tabla y el verbo existen",
    "tabla": "la tabla existe · falta el verbo",
    "otro": "existe con OTRO alcance",
    "no": "el concepto NO existe aqui",
    "legacy": "⛔ es la celda: decidido NO migrar",
}
CATALOGO = [
    ("org:leer", "hay"), ("org:renombrar", "tabla"),
    ("persona:listar", "tabla"), ("persona:ver", "tabla"),
    ("persona:invitar", "hay"), ("persona:retirar", "tabla"),
    ("persona:suprimir", "no"),
    ("invitacion:revocar", "tabla"),
    ("rol:conceder", "otro"), ("rol:revocar", "otro"),
    ("rol:listar-asignaciones", "tabla"),
    ("actividad:leer-propia", "tabla"), ("actividad:leer-toda", "tabla"),
    ("agente:crear", "no"), ("agente:listar", "no"),
    ("agente:retirar", "no"), ("agente:rotar", "no"),
    ("grupo:crear", "no"), ("grupo:renombrar", "no"), ("grupo:borrar", "no"),
    ("grupo:listar", "no"), ("grupo:anadir-miembro", "no"),
    ("grupo:quitar-miembro", "no"),
    ("token:emitir-propio", "no"), ("token:listar-propios", "no"),
    ("token:revocar-propio", "no"), ("token:listar-ajenos", "no"),
    ("token:revocar-ajenos", "no"),
    ("celda:listar", "legacy"), ("celda:crear", "legacy"), ("celda:retirar", "legacy"),
]
cuenta = {}
for nombre, e in CATALOGO:
    cuenta[e] = cuenta.get(e, 0) + 1
for e in ("hay", "tabla", "otro", "no", "legacy"):
    ns = [n for n, x in CATALOGO if x == e]
    print("  %-7s %2d  %s" % (e, len(ns), ESTADO[e]))
    print("           %s" % ", ".join(ns))

print("""
  ⇒ De 31, **%d no tienen concepto aqui** y **%d son la celda**, que ya se
    decidio no migrar. Solo %d funcionan hoy.

  ⛔ Y las dos que dicen «otro alcance» son las importantes: `rol:conceder` en
    su catalogo inviste a alguien EN LA ORGANIZACION; el nuestro concede sobre
    un RECURSO. Mismo nombre, planos distintos — es exactamente lo que la `011`
    acaba de separar. Adoptar su catalogo tal cual volveria a juntarlos.

  ── Lo que la pantalla necesita, medido ───────────────────────────────────
""" % (cuenta.get("no", 0), cuenta.get("legacy", 0), cuenta.get("hay", 0)))

rv = leer(CONSOLA / "components" / "governance" / "roles" / "RolesView.tsx")
for l in rv.splitlines():
    if "porDefecto.length" in l or "porRol[tab]?.length" in l or "Pertenecer ya da" in l:
        print("    RolesView.tsx:  " + l.strip()[:100])

print("""
  ⇒ La pantalla pinta DOS listas: lo que da pertenecer, y lo que AÑADE cada rol.
    Sin catalogo no hay nada que pintar — no es que se vea peor: es que la
    pantalla no tiene contenido.

  ── LAS TRES SALIDAS ──────────────────────────────────────────────────────

    ① adoptar un catalogo de potestades NUESTRO
       Escribir las que SI tienen referente y crecerlo con cada verbo.
       + la pantalla funciona, y el catalogo no miente
       − una lista mas que mantener, y `002` avisa: crece sin decision y deja
         de significar nada

    ② que el catalogo salga de lo que HAY
       Derivarlo del codigo: cada ruta declara que rol minimo exige, y el
       catalogo es esa tabla leida. P2 — lo derivable no se declara.
       + imposible que diverja, que es justo el motivo de su diseño
       − hay que escribirlo, y hoy `exige(...)` lleva el minimo suelto

    ③ cambiar la pantalla
       Pintar la escalera de cuatro con su `que_puede`, y no potestades.
       + cero codigo nuevo en `ore-iam`; ya esta en la base
       − se pierde «que añade cada rol», que es la pregunta que se hace quien
         va a conceder
""")

# ══════════════════════════════════════════════════════════════════════════
titulo("ⓒ", "LA ACTIVIDAD — y aqui la diferencia es de PROPOSITO, no de campos")
# ══════════════════════════════════════════════════════════════════════════

hu = leer(ORE / "iam" / "migraciones" / "008-la-huella.sql")
cols_hu = re.findall(r"^\s{2}(\w+)\s+\w", hu, re.M)
print("  iam.huella      %s" % ", ".join(cols_hu))
print("  rubix.acceso    id, organizacion, sujeto, actor, operacion, servidas,")
print("                  consideradas, servida_en   · PARTICIONADO POR DIA")

av = leer(CONSOLA / "components" / "governance" / "activity" / "ActivityView.tsx")
RUIDO = {"floor", "trim", "map", "length", "toLowerCase"}  # metodos de JS, no campos
usa = sorted(set(re.findall(r"h\.(\w+)", av)) - RUIDO)
print("\n  ActivityView lee:  %s" % ", ".join(usa))
faltan = [c for c in usa if c not in cols_hu and c not in ("nombre", "correo")]
print("  ⇒ de esos, `iam.huella` NO tiene: %s" % ", ".join(faltan))

print("""
  ⭐ `servidas` y `consideradas` no son adorno. Su propio fichero lo dice: son
    LA DECISION — cuantas filas se sirvieron de cuantas se miraron. Es lo unico
    que distingue «te enseñe todo» de «te filtre». Una huella administrativa no
    tiene ese numero porque no filtra nada: invitar es invitar.

  ⇒ Son dos registros distintos, no dos formas del mismo:

      rubix.acceso   quien vio QUE DATOS   · 400 dias · particionado · se suelta
      iam.huella     quien hizo QUE ACTO   · sin retencion escrita

  ⚠️ Y de paso, un hueco NUESTRO que sale al comparar: `iam.huella` **no tiene
    retencion**. La suya son 400 dias, y el numero no es suyo — es el que Google
    da a sus Admin Activity. La nuestra crece para siempre, y «para siempre» no
    es un plan: es una factura creciendo.

  ── LAS TRES SALIDAS ──────────────────────────────────────────────────────

    ① la pantalla pasa a ser LA HUELLA
       Se renombra a «actividad administrativa» y pinta actos, sin servidas.
       + existe hoy; solo falta la ruta
       − deja de contestar «quien vio mis datos», que es la pregunta cara

    ② se deja fuera hasta que exista el registro de accesos a datos
       Ese registro es de ORE —quien consulto que vista—, no de `iam`.
       + honesto, y no mancha `iam` con algo que no es suyo
       − una pestaña menos en la consola

    ③ las dos, separadas
       «Actividad» = accesos a datos (de ORE) · «Auditoria» = huella (de `iam`).
       + es lo que son, y cada una en su plano
       − dos pantallas y un registro que todavia no existe

  ── Y la pregunta que faltaba, MEDIDA ─────────────────────────────────────

    ¿Existe hoy en ORE un registro de quien consulto que vista? **No.**
    `ore-log` es el log de TRANSPARENCIA —solo crece, anota entradas y sirve
    pruebas de consistencia— y su propia cabecera dice que **no decide nada**:
    es para paquetes y firmas, no para accesos. No hay ninguna otra tabla ni
    fichero que registre una consulta.

    ⇒ ⓒ③ no es una opcion hoy: la mitad que le toca a ORE no existe. Queda ⓒ①
      ahora —y decir en la pantalla que son actos administrativos, no accesos—
      o ⓒ② y una pestaña menos.
""")

titulo("⇒", "LO QUE HAY QUE DECIDIR, en orden de cuanto desbloquea")
print("""
  1  ⭐ ¿una persona puede pertenecer a DOS clientes?
     Si NO: ⓐ① y la consola cabe casi tal cual. Si SI: hay selector, y hay que
     elegir entre ⓐ② y ⓐ③ (y ⓐ③ hay que medirlo antes contra este Keycloak).

  2  el catalogo: ⓑ② —derivarlo— es lo que encaja con P2, y es el unico que no
     puede divergir. Pero hoy no existe: `exige(...)` lleva el minimo suelto en
     cada ruta.

  3  la actividad: MEDIDO — ORE no registra accesos a datos, y `ore-log` es otra
     cosa. ⇒ ⓒ③ no esta disponible. O ⓒ① con la pantalla diciendo lo que es, o
     ⓒ② y una pestaña menos. Y de paso: `iam.huella` no tiene retencion.
""")
