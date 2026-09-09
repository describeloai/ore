# -*- coding: utf-8 -*-
"""MEDIDA · el asistente de fuentes, y qué hace falta para que deje de ser mock.

No es «conectar un botón». La pregunta de verdad es cuánto de lo que ORE sabe
hacer llega hoy a la interfaz, y qué es lo que impide que llegue.

Se mide sobre los ficheros:

    C:\\rubix-platform\\components\\ingestion\\wizard\\   los cuatro pasos
    C:\\ORE\\crates\\ore-serve\\src\\rutas.rs             lo que el servidor acepta
    C:\\ORE\\crates\\ore-serve\\src\\mando.rs             lo que puede correr

    uso:  python pruebas-de-fuego/medida-el-asistente-de-fuentes.py
"""
import pathlib
import re
import sys

try:
    sys.stdout.reconfigure(encoding="utf-8")
except AttributeError:
    pass

CONSOLA = pathlib.Path(r"C:\rubix-platform")
ORE = pathlib.Path(r"C:\ORE")
hallazgos = []


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
titulo("①", "QUÉ RECOGE EL ASISTENTE, y qué hace con ello")
# ══════════════════════════════════════════════════════════════════════════

w = leer(CONSOLA / "components" / "ingestion" / "wizard" / "SourceSetupWizard.tsx")
cfg = re.search(r"export interface SourceConfig \{(.*?)\}", w, re.S)
cred = re.search(r"EMPTY_CREDENTIALS[^=]*=\s*\{(.*?)\}",
                 leer(CONSOLA / "components" / "ingestion" / "wizard" / "ConnectionStep.tsx"), re.S)

print("\n  Los cuatro pasos: overview · name · connection · summary\n")
print("  Lo que junta:")
for l in (cfg.group(1) if cfg else "").strip().splitlines():
    print("      %s" % l.strip())
print("\n  Y las credenciales del paso 3:")
print("      %s" % re.sub(r"\s+", " ", (cred.group(1) if cred else "").strip()))

print("""
  ── ⛔ Y lo que hace el paso 4 ─────────────────────────────────────────────""")
for l in w.splitlines():
    if "setTimeout" in l:
        print("      %s" % l.strip())
print("""
  ⇒ Las DOS cosas son `setTimeout`. No sólo el alta: **la prueba de conexión
    tambien**, y ademas solo mira que el campo `host` no este vacio. Una
    pantalla que dice «Verificando conexion…» y no verifica nada es peor que no
    tenerla: entrena a creerla.""")
hallazgos.append("el alta Y la prueba de conexion son `setTimeout`")

# ══════════════════════════════════════════════════════════════════════════
titulo("②", "QUÉ ACEPTA `POST /fuentes`, que ya existe y funciona")
# ══════════════════════════════════════════════════════════════════════════

r = leer(ORE / "crates" / "ore-serve" / "src" / "rutas.rs")
# ⚠️ SOLO el cuerpo de `alta_de_fuente`. Barrer el fichero entero mezclaba esta
#   funcion con `fuentes`, que LEE — y colaba `connectionEnv` como si fuera algo
#   que se manda al dar de alta. Una medida que cruza dos funciones inventa un
#   contrato que no existe.
alta = r[r.index("fn alta_de_fuente"):]
alta = alta[:alta.index("\n    fn ")]
campos = sorted(set(re.findall(r'campo\("(\w+)"\)', alta)))
# ⛔ Anclado al principio de linea: `if let Some(t) = campo("type")` contiene
#   la misma subcadena, y sin el ancla los OPCIONALES salian como obligatorios.
obligatorios = sorted(set(re.findall(
    r'^\s*let Some\(\w+\) = campo\("(\w+)"\)', alta, re.M)))
print("\n  Cuerpo de `POST /fuentes`:  %s" % ", ".join(campos))
print("  Obligatorios:               %s" % ", ".join(obligatorios))
print("""
  Y con eso corre, DENTRO del plano de control y contra la forja:

      ore source add --name <nombre> <url> [--type] [--description]

  ⇒ El alta funciona hoy. El commit lo firma quien pulsa, y el arbol crece en
    el repositorio del inquilino. Eso no hay que construirlo.""")

# ══════════════════════════════════════════════════════════════════════════
titulo("③", "⛔⛔ EL CHOQUE, Y NO ES DE FORMATO: ES DE DISEÑO")
# ══════════════════════════════════════════════════════════════════════════

m = re.search(r"//! - \*\*No acepta una URL con credencial dentro\.\*\*(.*?)//! - ", r, re.S)
motivo = re.sub(r"\n//!\s*", " ", m.group(1)).strip() if m else ""

print("""
  El asistente recoge CINCO campos de credencial —host, port, database,
  username, password— y la unica forma de meterlos en el `url` que
  `POST /fuentes` pide es componer `postgres://usuario:clave@host/base`.

  ⛔ Y eso es exactamente lo que el servidor RECHAZA, a proposito:
""")
print("      «%s»" % motivo[:400])
print("""
  ⇒ No es que falte fontaneria. **El asistente asume que la credencial viaja
    con la fuente.** Componerla dentro de la URL es la unica forma de mandar
    esos cinco campos, y es justo el atajo que el servidor niega.

  ⭐⭐ Y AL MEDIRLO SALE QUE LA MITAD YA ESTA DECIDIDA.

    `ore source add` NO guarda la credencial en el manifiesto: la separa y
    escribe el NOMBRE de una variable —

        - name: pg
          type: postgres
          connectionEnv: PG_URL

    — de modo que el arbol dice **donde esta el secreto, no cual es**. Por eso
    `GET /fuentes` puede devolver `connectionEnv` sin devolver nada sensible, y
    por eso el manifiesto se versiona en git sin pensarlo dos veces.

    ⇒ La pregunta que queda NO es «donde viven los secretos» en abstracto. Es
      mucho mas pequeña: **quien escribe el VALOR de esa variable, y donde**.
      Un `Secret` de Kubernetes inyectado en el Job que corre `source catalog`
      contesta las dos, y ese Job ya existe.

    ⚠️ Lo que `ore-serve` rechaza es el atajo —mandar la URL con la clave dentro
      y dejar que acabe en un fichero del pod—, no la idea. Su negativa trae la
      condicion de levantamiento escrita: *«el dia que haya un sitio de verdad
      donde ponerla»*. Ese sitio existe; lo que falta es usarlo.""")
hallazgos.append("⭐ el arbol ya dice DONDE esta el secreto (`connectionEnv`): media decision tomada")

# ══════════════════════════════════════════════════════════════════════════
titulo("④", "LO QUE ORE SABE HACER, Y CUÁNTO LLEGA A LA INTERFAZ")
# ══════════════════════════════════════════════════════════════════════════

mando = leer(ORE / "crates" / "ore-serve" / "src" / "mando.rs")
hermeticos = re.findall(r'"([a-z]+)"', mando.split("HERMETICOS")[-1][:400]) if "HERMETICOS" in mando else []
rutas = re.findall(r'\("(GET|POST)", "(/[^"]+)"', r)

print("\n  El servidor sirve:")
for met, ruta in rutas:
    print("      %-5s %s" % (met, ruta))

print("\n  Y en la consola, pantallas que consuman algo de eso:")
encontrado = False
for p in list((CONSOLA / "app").rglob("*.tsx")) + list((CONSOLA / "lib").rglob("*.ts")):
    t = leer(p)
    # ⛔ Las RUTAS, no palabras sueltas: «decisiones» aparece en prosa y daba
    #   dos falsos positivos. Una medida que cuenta de mas es tan mala como una
    #   que cuenta de menos.
    if "/fuentes" in t or "/paquetes" in t:
        print("      %s" % p.relative_to(CONSOLA))
        encontrado = True
if not encontrado:
    print("      — NINGUNA —")

print("""
  ⇒ `ore-serve` sirve siete rutas y la consola no llama a ninguna. Y las que
    NO tienen pantalla son justo las que hacen que esto sea un producto y no un
    formulario:

      GET  /paquetes                    que hay en el arbol
      GET  /paquetes/{n}/decisiones     LA COLA DE `review`
      POST /paquetes/{n}/decisiones     contestarla

  ⭐ `discover` induce una ontologia y deja preguntas que alguien tiene que
    contestar. **Esa cola es el producto**: es donde una persona decide que
    significan sus datos, y hoy no tiene ni una pantalla. Enchufar solo
    `source add` deja el asistente terminando en un sitio sin salida.""")
hallazgos.append("7 rutas servidas, 0 consumidas · y `review` no tiene pantalla")

# ══════════════════════════════════════════════════════════════════════════
titulo("⑤", "¿Y EL ÁRBOL? — de donde sale, y por que `ore init` NO va en el login")
# ══════════════════════════════════════════════════════════════════════════

print("""
  `ore-serve` CLONA la forja en cada peticion y empuja la que escribe. Si la
  organizacion no tiene repositorio, el asistente falla en el ultimo paso, que
  es el peor sitio donde enterarse.

  ⛔ Y el disparador NO es entrar. Entrar es autenticarse; crear un arbol es
    aprovisionar, y son la misma frontera que ya separa `admitir` de `fundar`:

      · el arbol es de la ORGANIZACION, y desde `0021` una persona puede estar
        en varias ⇒ «al entrar» ni siquiera nombra un arbol
      · un login pasa cientos de veces y una fundacion una: colgar un acto de
        provision de un evento repetido obliga a que sea idempotente **y a que
        nadie se entere el dia que deje de serlo**
      · y quien entra sin pertenecer a ninguna no tiene donde inicializar nada

  ⇒ Va DENTRO de fundar, por su propio argumento: *«una organizacion sin
    administrador es una organizacion huerfana desde el primer segundo»*. Una
    sin arbol es lo mismo, y se entera mas tarde.""")
hallazgos.append("`ore init` va en fundar, no en el login")

# ══════════════════════════════════════════════════════════════════════════
titulo("⇒", "LO QUE SALE DE MEDIR")
# ══════════════════════════════════════════════════════════════════════════
for h in hallazgos:
    print("  · " + h)
print("""
  El orden, y el primero NO es escribir codigo:

    0  ⭐ QUIEN ESCRIBE el valor de `connectionEnv`, y donde   ← lo que bloquea
    1  el arbol se crea al FUNDAR, no al entrar
    2  el paso 3 deja de mentir: o prueba de verdad, o no lo dice
    3  el paso 4 llama a `POST /fuentes` — sin clave dentro
    4  `discover` y LA COLA DE DECISIONES, que es el producto
    5  el catalogo: `GET /paquetes`

  ⚠️ El 0 es mas pequeño de lo que parecia. El manifiesto ya esta resuelto —dice
    el NOMBRE de la variable, no su valor— asi que lo que falta es que el
    asistente mande `connectionEnv` en vez de una URL con la clave dentro, y que
    alguien escriba un `Secret` con el valor.

  ⭐ Y se puede partir: **el alta funciona sin credencial**. Dar de alta una
    fuente, que el arbol crezca y que el commit lo firme quien pulso el boton ya
    es util por si solo. Lo que no puede hacerse sin el secreto es `source
    catalog` —mirar dentro del origen—, que es el paso siguiente y no este.

  ⇒ El 3 se puede hacer YA, sin esperar al 0, si el asistente deja de pedir una
    contraseña que no puede entregar a nadie.""")
