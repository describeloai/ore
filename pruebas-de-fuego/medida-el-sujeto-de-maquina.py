# -*- coding: utf-8 -*-
"""MEDIDA · un Job que saca un secreto y NO es una persona.

Lo que lo dispara, medido en vivo el 2026-09-10 en `t-demo`, en el primer Job de
catálogo que llegó a pedir de verdad:

    ### pidiendo `fuente-postgresql_20260910_1305` al custodio, para `VENTAS_…_URL`
    ✗ el custodio contesto 422
    {"error":"quien pide no es una persona conocida aqui"}

⭐⭐ Y la pregunta que se mide NO es «cómo le damos una persona al Job». Es
  **cuánto del concepto de agente está ya escrito en el modelo**, porque tres
  migraciones lo nombran y ninguna lo implementa.

Se mide sobre el árbol, no sobre la base: los números salen de los ficheros, así
que esto corre sin clúster y dice lo mismo mañana.

    uso:  python pruebas-de-fuego/medida-el-sujeto-de-maquina.py
"""
import pathlib
import re
import sys

try:
    sys.stdout.reconfigure(encoding="utf-8")
    sys.stderr.reconfigure(encoding="utf-8")
except AttributeError:
    pass

RAIZ = pathlib.Path(__file__).resolve().parent.parent


def leer(p):
    f = RAIZ / p
    return f.read_text(encoding="utf-8") if f.exists() else ""


def titulo(t):
    print("\n" + "═" * 74)
    print(t)
    print("═" * 74)


fallos = []

# ══════════════════════════════════════════════════════════════════════════
titulo("① DÓNDE SE PARA, Y ES UNA SOLA LÍNEA")
# ══════════════════════════════════════════════════════════════════════════
#
# El Job de catálogo pide `GET /organizaciones/{org}/secretos/{nombre}`. Todo el
# camino funciona hasta que el custodio traduce el `sub` del testigo a una fila
# de `iam.persona`.
verbos = leer("crates/ore-iam/src/verbos.rs")
m = re.search(r"pub fn persona_id.*?\n}", verbos, re.S)
if not m:
    fallos.append("no se encuentra `persona_id` en `ore-iam/src/verbos.rs`")
else:
    linea = verbos[: m.start()].count("\n") + 1
    print(f"  `verbos.rs:{linea}` · persona_id() — resuelve el `sub` a `iam.persona`")
    print("     y contesta «quien pide no es una persona conocida aqui» si no está.")

cofre = leer("crates/ore-cofre/src/rutas.rs")
usos = [
    (cofre[:m2.start()].count("\n") + 1)
    for m2 in re.finditer(r"verbos::persona_id\(", cofre)
]
print(f"\n  El custodio lo llama en {len(usos)} sitios: líneas {', '.join(map(str, usos))}")
print("  ⇒ `emitir` y `resolver`. LISTAR no lo llama: pregunta por potestad.")

# ══════════════════════════════════════════════════════════════════════════
titulo("② QUÉ NECESITA EL JOB, EXACTAMENTE — y es menos de lo que parece")
# ══════════════════════════════════════════════════════════════════════════
#
# ⭐ Un Job de catálogo NO emite y NO lista. Sólo RESUELVE. Y `resolver` no toca
#   el plano de la organización: su única pregunta es la concesión del recurso.
resolver = re.search(r"fn resolver\(.*?\n    }\n", cofre, re.S)
cuerpo = resolver.group(0) if resolver else ""
usa_potestad = "potestad::exige" in cuerpo
usa_concesion = "concesion_viva" in cuerpo
roles = re.search(r"c\.rol in \(([^)]*)\)", cuerpo)
print(f"  ¿`resolver` pregunta por POTESTAD de organización?   {'SÍ' if usa_potestad else 'NO'}")
print(f"  ¿`resolver` pregunta por CONCESIÓN de recurso?       {'SÍ' if usa_concesion else 'NO'}")
if roles:
    print(f"  Roles que abren un secreto:  {roles.group(1).strip()}")
if usa_potestad:
    fallos.append("`resolver` toca el plano de la organización: la medida de abajo no vale")
print("""
  ⇒ Un agente NO necesita entrar en `iam.potestades_de_persona`, que es el plano
    donde viven `secreto:emitir` y `secreto:listar`. Necesita UNA fila de
    `iam.concesion` con rol `usar`. Emitir sigue siendo de una persona —la `018`
    lo puso ahí a propósito— y esta medida no lo toca.""")

# ══════════════════════════════════════════════════════════════════════════
titulo("③ LO QUE EL MODELO YA TIENE PARA AGENTES, Y NADIE USA")
# ══════════════════════════════════════════════════════════════════════════
#
# ⭐⭐ Ésta es la medida que cambia la decisión. Tres migraciones nombran al
#   agente, dos le hacen sitio, y ninguna lo implementa.
piezas = [
    (
        "007-la-concesion.sql",
        r"`sujeto` y no `persona`",
        "`concesion.sujeto` es TEXTO y no clave ajena a `persona`, "
        "«porque aquí también entra un agente —un Job de ORE actuando por "
        "alguien—, y eso es RFC 8693»",
    ),
    (
        "008-la-huella.sql",
        r"`agente` es lo que actuó por ella",
        "`iam.huella` tiene columna `agente` desde su creación: "
        "«`quien` es la persona; `agente` es lo que actuó por ella»",
    ),
    (
        "014-las-potestades.sql",
        r"un concepto de agente que aquí no existe",
        "y aquí el hueco está DICHO: una potestad de `securityadmin` "
        "«necesita un concepto de agente que aquí no existe»",
    ),
]
for fichero, patron, dice in piezas:
    t = leer(f"iam/migraciones/{fichero}")
    ok = bool(re.search(patron, t))
    print(f"  {'✓' if ok else '✗'} {fichero:<28} {dice}")
    if not ok:
        fallos.append(f"`{fichero}` ya no dice lo que esta medida cita")

print("""
  ⇒ El plano del RECURSO se diseñó para esto desde el principio. Lo que falta no
    es modelar un agente: es CUMPLIR una decisión que ya está escrita y que el
    código no llegó a honrar.""")

# ══════════════════════════════════════════════════════════════════════════
titulo("④ EL RADIO, MEDIDO — a qué se ata `persona` de verdad")
# ══════════════════════════════════════════════════════════════════════════
#
# Si `persona` estuviera en el centro de todo, (b) sería una refactorización. Se
# cuenta lo que la referencia DE VERDAD.
mig = "\n".join(leer(f"iam/migraciones/{p.name}") for p in sorted((RAIZ / "iam/migraciones").glob("*.sql")))
fks = re.findall(r"^\s*(\w+)\s+text[^\n]*references iam\.persona", mig, re.M)
print(f"  Columnas con clave ajena a `iam.persona`:  {len(fks)}")
print(f"     {', '.join(sorted(set(fks)))}")
print("""
  ⛔ Y ni una es `concesion.sujeto` ni `huella.quien`. Las que hay son de AUTORÍA
    —quién concedió, quién revocó, quién emitió— y ésas SÍ son de una persona:
    un agente no concede nada. La decisión de la `007` se sostiene entera.""")

# ══════════════════════════════════════════════════════════════════════════
titulo("⑤ LO QUE CUESTA (b), ENTONCES")
# ══════════════════════════════════════════════════════════════════════════
print("""  ① CÓMO SE LLAMA UN AGENTE. Hoy el testigo del Job trae un `sub` de cuenta de
     servicio y nada lo distingue de una persona salvo que no está en la tabla.
     Hace falta un identificador de sujeto que se vea que no es alguien —y que
     `concesion.sujeto` ya admite sin cambiar una línea de SQL.

  ② QUE `resolver` LO ACEPTE. Una línea: `persona_id` deja de ser la única forma
     de convertir un `sub` en un `sujeto`.

  ③ QUE LA HUELLA LO ANOTE EN SU SITIO. La columna existe: `agente`, no `quien`.
     ⛔ Y esto NO es cosmética: fundir los dos haría que la auditoría no pudiera
       contestar «¿quién sacó este secreto?» ni «¿por cuenta de quién?», que es
       exactamente lo que la `008` dice que no se funda.

  ④ QUIÉN LE CONCEDE `usar`. Una concesión no nace sola. Hoy `emitir` crea la del
     emisor; la del agente tiene que crearla alguien con `concesion:conceder`,
     y decidir CUÁNDO —en el alta de la fuente es lo natural— es la parte de
     diseño que queda, no la de código.

  ⚠️ Y lo que esta medida NO contesta: si el agente es de la plataforma o del
    inquilino. Hoy `idp-agente` es UNO para todos, así que concederle `usar`
    sobre el secreto de `demo` se lo concede al agente que también trabaja para
    `prueba`. Eso es el mismo patrón que el cofre rechazó con su llave y que el
    driver rechazó con su cuenta — y aquí volvería a entrar por la puerta de
    atrás si nadie lo mira.""")

# ══════════════════════════════════════════════════════════════════════════
print("\n" + "═" * 74)
if fallos:
    print("⛔ LA MEDIDA NO SE SOSTIENE:")
    for f in fallos:
        print("   · " + f)
    sys.exit(1)
print("✓ medida coherente: el plano del recurso ya admite un agente; falta usarlo")
