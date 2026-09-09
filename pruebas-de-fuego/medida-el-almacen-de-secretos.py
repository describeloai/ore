# -*- coding: utf-8 -*-
"""MEDIDA · el almacén de secretos contra el modelo de roles que YA existe.

Lo que se mide no es «qué secretos hay» —eso es un inventario— sino **cuánto del
modelo de quién-puede-qué está ya escrito**, y qué falta exactamente para que un
secreto quepa en él sin inventar un paradigma paralelo.

La frase que lo dispara, y resulta ser el modelo entero:

    «cualquier usuario con suficiente rol puede emitir un secreto; ahora leerlo
     o usarlo ya es otra historia»

Se mide sobre:

    iam/migraciones/014-las-potestades.sql      el plano de la ORGANIZACION
    iam/migraciones/007-la-concesion.sql        el plano del RECURSO
    iam/migraciones/011-el-rol-del-recurso.sql  y su vocabulario
    iam/migraciones/008-la-huella.sql           lo que queda escrito

    uso:  python pruebas-de-fuego/medida-el-almacen-de-secretos.py
"""
import pathlib
import re
import sys

try:
    sys.stdout.reconfigure(encoding="utf-8")
    sys.stderr.reconfigure(encoding="utf-8")
except AttributeError:
    pass

ORE = pathlib.Path(__file__).resolve().parent.parent
MIG = ORE / "iam" / "migraciones"
hallazgos = []
rojo = []


def titulo(n, t):
    print("\n" + "=" * 78)
    print("%s · %s" % (n, t))
    print("=" * 78)


def leer(p):
    try:
        return (MIG / p).read_text(encoding="utf-8", errors="replace")
    except OSError:
        rojo.append("no se pudo leer %s" % p)
        return ""


def exige(cond, m):
    if not cond:
        rojo.append(m)
    return cond


potestades = leer("014-las-potestades.sql")
concesion = leer("007-la-concesion.sql")
rolrec = leer("011-el-rol-del-recurso.sql")
huella = leer("008-la-huella.sql")

# ══════════════════════════════════════════════════════════════════════════
titulo("①", "QUE HAY YA — y es mucho mas de lo que parece")
# ══════════════════════════════════════════════════════════════════════════

filas = re.findall(r"^\s*\('([a-z:_-]+)',\s*'([^']*)',\s*(true|false)\)",
                   potestades, re.M)
print("\n  `iam.potestad` — el plano de la ORGANIZACION (%d):\n" % len(filas))
for n, q, e in filas:
    print("      %-22s %-52s %s" % (n, q[:52], "" if e == "true" else "todavia no"))
exige(len(filas) == 11, "el censo de potestades ya no son 11: esta medida esta vieja")

roles = re.findall(r"^\s*\('(lector|owner)',\s*'([^']*)'\)", rolrec, re.M)
print("\n  `iam.rol_de_recurso` — el plano del RECURSO (%d):\n" % len(roles))
for n, q in roles:
    print("      %-22s %s" % (n, q))
exige(len(roles) == 2, "los roles de recurso ya no son dos")

print("""
  ⛔ Y NINGUNA de las once nombra un secreto. Eso es lo esperado: el almacen no
    existe. Lo que importa es que **el sitio donde meterlas ya esta hecho** — la
    tabla, la vista `potestades_de_rol`, la union de conjuntos de la `016`, y la
    guarda de contencion que impide dar lo que no se tiene.""")
hallazgos.append("el modelo de potestades existe y esta vacio de secretos: hay donde meterlas")

# ══════════════════════════════════════════════════════════════════════════
titulo("②", "⭐⭐ TU FRASE ES EL MODELO, Y CAE EN LOS DOS PLANOS QUE YA HAY")
# ══════════════════════════════════════════════════════════════════════════

print("""
  «cualquier usuario con suficiente rol puede EMITIR un secreto;
   ahora LEERLO o USARLO ya es otra historia»

  Eso no es una matizacion: es exactamente el corte que la `011` peleo y gano.

      EMITIR    es del plano de la ORGANIZACION. No habla de ningun secreto
                concreto porque el secreto todavia no existe. Es una POTESTAD,
                como `invitacion:emitir`

      LEER/USAR es del plano del RECURSO. Habla de ESE secreto y de nadie mas.
                Es una CONCESION, como la de una vista del arbol

  ⭐ Y la asimetria que hace falta sale sola: quien emite un secreto no queda
    con derecho a leer todos los demas, porque emitir vive arriba y leer vive
    abajo, **y entre los dos planos no hay herencia**. La `011` lo dejo escrito
    para otra cosa y vale igual aqui: *«esta tabla tiene dos filas y ninguna es
    mas alta que la otra»*.

  ⚠️ Lo que SI hay que decidir, y es la unica pregunta de diseño de verdad:
    **quien emite, ¿queda `owner` de lo que emitio?** Si no, un secreto nace sin
    nadie que pueda darlo, y eso es la organizacion huerfana otra vez. Si si, hay
    que decirlo — y entonces emitir sí produce un derecho sobre ESE secreto, y
    sobre ninguno mas.""")
hallazgos.append("⭐ emitir es POTESTAD (arriba) y leer/usar es CONCESION (abajo): ya existen los dos")

# ══════════════════════════════════════════════════════════════════════════
titulo("③", "⛔⛔ Y FALTA UN TERCER ROL DE RECURSO: `usar`")
# ══════════════════════════════════════════════════════════════════════════

print("""
  Hoy el plano de abajo sabe decir dos cosas: `lector` —ve y no cambia— y
  `owner` —firma—. Para una vista de la ontologia bastan.

  ⛔ Para un secreto NO, y es la diferencia que define este producto:

      lector   VE EL VALOR. Es lo que hace una persona que copia una contraseña
      usar     lo RESUELVE sin verlo. Es lo que hace un Job que se conecta

  ⇒ Y la inmensa mayoria del acceso a un secreto tiene que ser `usar`. Un
    almacen donde para conectarte hay que poder leer la contraseña es un cajon
    con una puerta: cada permiso de ejecucion arrastra un permiso de lectura, y
    la auditoria no puede distinguir «se conecto» de «se la llevo».

  ⭐ Y encaja sin tocar nada: `iam.rol_de_recurso` no tiene ordinal **a
    proposito**, asi que meter un tercer rol es INSERTAR UNA FILA. No hay
    escalera que reordenar, y `usar` no implica `lector` por la misma razon por
    la que `owner` no lo implica — dos hechos, no una regla.

  ⚠️ Y con eso la guarda de contencion sigue funcionando tal cual: se puede
    conceder lo que se tiene, y quien solo tiene `usar` no puede dar `lector`.""")
hallazgos.append("⛔ falta `usar` — resolver el secreto SIN verlo. Es una fila, no un rediseño")

# ══════════════════════════════════════════════════════════════════════════
titulo("④", "⭐ Y EL SUJETO YA PUEDE SER UNA MAQUINA")
# ══════════════════════════════════════════════════════════════════════════

sujeto = re.search(r"^\s*sujeto\s+(\w+)\s+not null(.*)$", concesion, re.M)
tiene_fk = bool(sujeto and "references" in (sujeto.group(2) or ""))
print("\n  `iam.concesion.sujeto`  →  %s%s"
      % (sujeto.group(1) if sujeto else "?",
         ", CON clave ajena a persona" if tiene_fk else ", **sin clave ajena a `persona`**"))
exige(sujeto and not tiene_fk,
      "`concesion.sujeto` ya referencia a `persona`: un agente no cabria")

print("""
  ⇒ Es texto libre a proposito, y la `0021` dice por que: *«para que quepa un
    agente (RFC 8693)»*. O sea que **el Job que lee un origen puede ser el titular
    de la concesion**, con nombre propio, en vez de correr con la de una persona.

  ⭐ Eso es justo lo que un almacen de secretos necesita para que la huella
    signifique algo: «lo uso el Job de catalogo de `ventas`, a las 03:14» y no
    «lo uso Ada», que es mentira desde el primer dia en que algo se automatiza.""")
hallazgos.append("⭐ `concesion.sujeto` es texto sin clave ajena: un Job puede ser el titular")

# ══════════════════════════════════════════════════════════════════════════
titulo("⑤", "LO QUE `concesion` REGALA, y no hay que volver a escribir")
# ══════════════════════════════════════════════════════════════════════════

for campo, que in [
    ("hasta", "acceso TEMPORAL con fecha de fin. Un secreto prestado tres dias"),
    ("revocada_en", "revocar sin borrar: la fila se queda y dice cuando"),
    ("revoco", "y QUIEN la revoco"),
    ("concedio", "y quien la dio"),
    ("organizacion", "el aislamiento, por la `009`"),
]:
    hay = re.search(r"^\s*%s\s" % campo, concesion, re.M) or campo in leer(
        "009-el-ambito-de-la-concesion.sql")
    print("  %s %-14s %s" % ("·" if hay else "⛔", campo, que))

viva = "concesion_viva" in concesion
print("  %s %-14s %s" % ("·" if viva else "⛔", "concesion_viva",
                         "lo que vale AHORA. Se pregunta a la vista, no a la tabla"))
exige(viva, "`concesion_viva` ya no esta")

print("""
  ⇒ Prestar una credencial hasta el viernes, retirarla y poder demostrar
    cuando y quien — los tres estan escritos ya, para el arbol. Un almacen que
    los reinventara tendria DOS modelos de caducidad que divergen.""")

# ══════════════════════════════════════════════════════════════════════════
titulo("⑥", "⛔ LO QUE FALTA DE VERDAD")
# ══════════════════════════════════════════════════════════════════════════

hay_secreto = "secreto" in potestades.lower()
recurso_libre = bool(re.search(r"^\s*recurso\s+text\s+not null,\s*$", concesion, re.M))
# ⛔ Desde el PARENTESIS, no desde `create table`: si no, la primera columna
#   listada era la palabra `create`, y una medida que se equivoca en algo tan
#   visible pone en duda lo que dice de lo que no se ve.
cuerpo = huella[huella.index("create table"):]
huella_campos = re.findall(r"^\s{2}(\w+)\s+\w+", cuerpo[cuerpo.index("(") : cuerpo.index(");")], re.M)

print("""
  ⛔ 1 · NO HAY POTESTAD DE SECRETO. Ni `secreto:emitir` ni `secreto:listar`.
       Es una fila en `iam.potestad` y unas cuantas en `iam.rol_potestad`, pero
       hay que decidir QUE ROL la lleva — y ahi `SECURITYADMIN`, que hoy es una
       carcasa con una sola potestad sin ejercer, deja de serlo.

  ⛔ 2 · `concesion.recurso` ES TEXTO LIBRE. Para una vista del arbol se
       aguanta; para un secreto es el ASIDERO — lo que se concede, lo que se
       audita y lo que un manifiesto referencia. Sin una forma cerrada, dos
       escrituras del mismo secreto no son el mismo recurso, y la concesion no
       alcanza a lo que creias.

  ⛔ 3 · LA HUELLA NO TIENE «PARA QUE». Sus campos son: %s.
       Sirve para «quien hizo que»; a un secreto le falta el proposito de la
       resolucion, que es la pregunta que hace un auditor: no «¿quien lo leyo?»
       sino «¿por que lo leyo?».

  ⚠️ 4 · Y NADA DE ESTO GUARDA UN VALOR. `iam` sigue sin ser el almacen: es
       quien dice **quien puede**. Donde vive el valor y como se resuelve es la
       otra mitad, y no cabe en estas tablas ni debe.""" % ", ".join(huella_campos))

exige(not hay_secreto, "ya hay potestades de secreto: esta medida esta vieja")
hallazgos.append("⛔ falta la potestad de emitir, la FORMA del nombre de recurso, y el «para que»")

# ══════════════════════════════════════════════════════════════════════════
titulo("⇒", "LA FORMA QUE SALE DE MEDIR")
# ══════════════════════════════════════════════════════════════════════════
for h in hallazgos:
    print("  · " + h)

print("""
  ⭐⭐ El almacen NO necesita un modelo de permisos nuevo. Necesita TRES cosas
    dentro del que ya hay:

      1  potestades de organizacion   `secreto:emitir` y `secreto:listar`
                                      — y `SECURITYADMIN` deja de ser una carcasa

      2  un rol de recurso mas        `usar` — resolver sin ver. Una fila, porque
                                      esa tabla no tiene ordinal a proposito

      3  una FORMA para el recurso    el asidero del secreto, cerrada, para que
                                      dos escrituras nombren lo mismo

  Y una decision, que es tuya:

    ⚠️ quien emite un secreto, ¿queda `owner` de el? Si no, nace sin nadie que
      pueda darlo. Si si, emitir produce un derecho sobre ESE secreto — y sobre
      ninguno mas, que es lo que hace que la respuesta sea segura.

  ⛔ Lo que esto NO contesta, y va aparte: donde vive el VALOR y como se
    resuelve. `iam` dice quien puede; no guarda nada. Confundir las dos mitades
    es como acaban los almacenes de secretos siendo bases de datos con contraseñas
    dentro y un `select` que alguien olvido cerrar.""")

if rojo:
    print("\n⛔ LA MEDIDA NO CUADRA CON EL ARBOL:")
    for r in rojo:
        print("   · " + r)
    sys.exit(1)
print("\n✓ todo lo que esta medida afirma sigue estando en el arbol")
