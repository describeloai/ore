# -*- coding: utf-8 -*-
"""MEDIDA · el censo de potestades, y qué tomar de sus tres roles.

Confirmado que las potestades son **del plano de gestión de la organización** y
no tocan el árbol ni la ontología, quedan dos preguntas que sólo contesta medir:

    ¿cuántas potestades tenemos DE VERDAD, sacadas de lo que hay escrito?
    ¿cuáles de sus tres roles tienen contenido AQUÍ, y cuáles serían un nombre
    vacío?

    uso:  python pruebas-de-fuego/medida-el-censo-de-potestades.py
"""
import pathlib
import re
import sys

try:
    sys.stdout.reconfigure(encoding="utf-8")
except AttributeError:
    pass

ORE = pathlib.Path(r"C:\ORE")
PLATAFORMA = pathlib.Path(r"C:\Rubix")


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
titulo("①", "YA ESCRIBIMOS POTESTADES. Estaban en la huella, sin tabla")
# ══════════════════════════════════════════════════════════════════════════

fuente = "".join(leer(ORE / "crates" / "ore-iam" / "src" / f)
                 for f in ("rutas.rs", "verbos.rs", "fundar.rs"))
anotadas = sorted(set(re.findall(r'"([a-z]+:[a-z]+)"', fuente)))

print("""
  `iam.huella.operacion` guarda el nombre del acto, y desde el primer verbo se
  escribió con la forma `recurso:verbo` — que es EXACTAMENTE la gramática de una
  potestad. Es decir: el vocabulario ya existe y es cerrado en la práctica; lo
  que falta es la tabla que lo haga cerrado de verdad y la columna que diga qué
  rol lo tiene.
""")
for a in anotadas:
    print("    %s" % a)
print("\n  ⇒ %d actos, ya nombrados, ya en producción." % len(anotadas))

# Y las que tienen columna en la base pero todavía no verbo.
PENDIENTES = [
    ("invitacion:revocar", "`006` tiene `revocada_en` y `revoco`; falta el verbo"),
    ("rol:conceder", "investir a alguien ya dentro. Hoy solo lo escribe `admitir`"),
    ("org:traspasar", "`002` lo nombra desde el principio: «dueno: ademas TRASPASA»"),
]
print("\n  Y tres mas que la BASE ya soporta y ningun verbo usa:\n")
for n, por in PENDIENTES:
    print("    %-20s %s" % (n, por))
print("\n  ⇒ El censo de salida son %d potestades. No 31." % (len(anotadas) + len(PENDIENTES)))

# ══════════════════════════════════════════════════════════════════════════
titulo("②", "SUS 31, RECLASIFICADAS — y me equivoque al clasificarlas antes")
# ══════════════════════════════════════════════════════════════════════════

print("""
  La vez pasada puse 16 en «el concepto NO existe aqui», y eso mezclaba dos
  cosas muy distintas. Confirmado que las potestades son del plano de gestion de
  la organizacion, los grupos, los agentes y los tokens **son de este plano**:
  no son vocabulario ajeno, son cosas que NO HEMOS CONSTRUIDO todavia.

  ⇒ Su catalogo no es una lengua extranjera. Es en buena parte **un plan**.
""")

SUYAS = {
    "org:leer": "hay", "org:renombrar": "falta",
    "persona:listar": "hay", "persona:ver": "falta",
    "persona:invitar": "hay", "persona:retirar": "falta", "persona:suprimir": "falta",
    "invitacion:revocar": "falta",
    "rol:conceder": "falta", "rol:revocar": "falta", "rol:listar-asignaciones": "hay",
    "actividad:leer-propia": "falta", "actividad:leer-toda": "falta",
    "agente:crear": "falta", "agente:listar": "falta",
    "agente:retirar": "falta", "agente:rotar": "falta",
    "grupo:crear": "falta", "grupo:renombrar": "falta", "grupo:borrar": "falta",
    "grupo:listar": "falta", "grupo:anadir-miembro": "falta",
    "grupo:quitar-miembro": "falta",
    "token:emitir-propio": "ajena", "token:listar-propios": "ajena",
    "token:revocar-propio": "ajena", "token:listar-ajenos": "ajena",
    "token:revocar-ajenos": "ajena",
    "celda:listar": "fuera", "celda:crear": "fuera", "celda:retirar": "fuera",
}
QUE = {
    "hay": "ya la tenemos, con otro nombre o el mismo",
    "falta": "de ESTE plano, y sin construir. Es plan, no vocabulario ajeno",
    "ajena": "⚠️ los tokens los emite y revoca KEYCLOAK, no nosotros",
    "fuera": "⛔ la celda: decidido no migrar",
}
for e in ("hay", "falta", "ajena", "fuera"):
    ns = [n for n, x in SUYAS.items() if x == e]
    print("  %-6s %2d  %s" % (e, len(ns), QUE[e]))
    print("         %s" % ", ".join(sorted(ns)))

print("""
  ⭐ Y las cinco de `token:` merecen su propia frase. En su plataforma el
    servicio emitia tokens de API propios, asi que revocarlos era suyo. Aqui el
    emisor es Keycloak: revocar una sesion es una llamada a la Admin API del
    realm, y **no tenemos esa credencial a proposito** —exigiria `manage-realm`,
    medido en 403—. Adoptarlas seria escribir potestades que este plano no puede
    ejercer.
""")

# ══════════════════════════════════════════════════════════════════════════
titulo("③", "SUS TRES ROLES — cual tiene contenido aqui y cual seria un nombre vacio")
# ══════════════════════════════════════════════════════════════════════════

SECURITY = ["actividad:leer-toda", "token:listar-ajenos", "token:revocar-ajenos", "agente:rotar"]
USER = ["persona:invitar", "invitacion:revocar", "persona:retirar", "persona:suprimir",
        "agente:crear", "agente:retirar", "agente:rotar", "grupo:crear", "grupo:renombrar",
        "grupo:borrar", "grupo:anadir-miembro", "grupo:quitar-miembro", "org:renombrar"]

def vivas(ps):
    return [p for p in ps if SUYAS.get(p) in ("hay", "falta")]

print("""
  ── USERADMIN vs ACCOUNTADMIN ─────────────────────────────────────────────

    Lo que los separa es UNA potestad: `rol:conceder`. Su motivo:
    *«no puede ascenderse a si mismo»*.
""")
print("    ⇒ La distincion es REAL aqui y HOY: ya tenemos `invitacion:emitir`")
print("      —dar de alta— e `rol:conceder` esta a un verbo de existir. Dos roles")
print("      con contenido distinto desde el primer dia.")
print("      De sus %d potestades de USERADMIN, %d son de este plano." % (len(USER), len(vivas(USER))))

print("""
  ── ⛔ SECURITYADMIN — y aqui la medida dice que NO, todavia ───────────────
""")
print("    Su contenido entero, y que pasa con cada una aqui:\n")
for p in SECURITY:
    print("      %-24s %s" % (p, QUE[SUYAS[p]]))
print("""
    ⇒ De las cuatro: dos son de Keycloak, una necesita un concepto de agente que
      no existe, y la cuarta —`actividad:leer-toda`— tiene tabla (`iam.huella`) y
      **no tiene ruta**.

    ⛔ Adoptar SECURITYADMIN hoy es adoptar un rol VACIO, y eso es exactamente lo
      que `002-el-papel.sql` prohibe: *«una lista que puede crecer sin una
      decision deja de significar nada»*. Un rol sin potestades no significa
      nada y ademas parece que si.

    ⭐ Pero la IDEA se toma entera, y se apunta cuando toca. Su motivo es
      operativo y sigue siendo cierto:

        «CORTAR una credencial filtrada es urgente y pasa a las 3 de la
         mañana. Exigir el rol omnipotente para una emergencia significa que
         la credencial omnipotente acaba circulando.»

    ⇒ Nace el dia que exista `actividad:leer-toda` con su ruta, o algo que este
      plano pueda cortar de verdad. No antes.
""")

print("""  ── Y `dueno` NO es suyo: es nuestro ──────────────────────────────────────

    Sus tres roles no tienen equivalente. `019` no tiene ninguna restriccion de
    unicidad: varios pueden ser ACCOUNTADMIN a la vez.

    El nuestro tiene un indice unico parcial y un motivo que ellos no
    escribieron —`002`—: *«alguien tiene que poder quedarse sin administradores
    y aun asi entrar. Una organizacion cuyo ultimo administrador se va sin
    traspasar es una organizacion perdida»*.

    ⇒ Se queda, y no como el peldaño mas alto de una escalera: como **el que
      tiene `org:traspasar`, y es UNO**.
""")

# ══════════════════════════════════════════════════════════════════════════
titulo("④", "EL MAPA QUE SALE, con lo que hay hoy")
# ══════════════════════════════════════════════════════════════════════════

print("""
    por pertenecer     organizacion:listar · miembro:listar
                       ⭐ es su `POR_DEFECTO`, y su argumento vale entero:
                          «pertenecer ya da lectura. Leer no es un rol»

    administrador      + invitacion:listar · invitacion:emitir
                         invitacion:revocar ⬜
                         concesion:conceder · concesion:revocar

    dueno              + rol:conceder ⬜ · org:traspasar ⬜
                       y es UNO

  ⛔⛔ Y FIJATE EN QUIEN NO ESTA: `lector` y `miembro`.

    `lector` no añade nada sobre pertenecer. Y `miembro` no tiene ni una
    potestad de este plano — su unico significado era *«responde la cola de
    review»*, que es el ARBOL. La `012` dejo esa pregunta abierta a proposito;
    el censo la contesta sola.

  ⇒ La escalera de cuatro se queda en **tres estados**: pertenecer,
    administrar, y ser dueño. Que es, sin buscarlo, la forma a la que llegaron
    ellos con tres roles — por el mismo motivo y con distinto vocabulario.

  ── Y la guarda del rodeo deja de ser una resta ────────────────────────────

    «Puedes otorgar un rol si sus potestades estan CONTENIDAS en las tuyas.»

    Mas expresivo que comparar ordinales, se comprueba igual de facil, y admite
    el dia de mañana un rol que corte sin dar de alta — que es justo lo que un
    ordinal no sabe representar.
""")
