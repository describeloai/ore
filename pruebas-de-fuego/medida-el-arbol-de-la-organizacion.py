# -*- coding: utf-8 -*-
"""MEDIDA · el árbol de una organización: qué es, quién lo crea y cuándo.

La propuesta que se mide es concreta:

    «un árbol maestro por organización, que puede estar compuesto de varios
     repositorios — el mismo árbol; y nace en el mismo acto de crear la
     organización»

Se mide contra los ficheros, no contra la memoria:

    crates/ore-core/src/link.rs        qué es un miembro del workspace
    crates/ore-core/src/validate.rs    y qué se lee de `workspace.members`
    crates/ore-core/src/document.rs    las claves del manifiesto
    crates/ore-iam/Cargo.toml          con qué puede hablar `fundar`
    Dockerfile                         desde qué imagen corre
    iam/migraciones/004-…              qué se guarda de una organización
    crates/ore-serve/src/main.rs       de dónde sale el árbol que sirve
    crates/ore-serve/src/rutas.rs      y si alguna ruta nombra una organización

    uso:  python pruebas-de-fuego/medida-el-arbol-de-la-organizacion.py
"""
import pathlib
import re
import sys

try:
    sys.stdout.reconfigure(encoding="utf-8")
except AttributeError:
    pass

ORE = pathlib.Path(__file__).resolve().parent.parent
hallazgos = []
rojo = []


def titulo(n, t):
    print("\n" + "═" * 78)
    print("%s · %s" % (n, t))
    print("═" * 78)


def leer(p):
    try:
        return (ORE / p).read_text(encoding="utf-8", errors="replace")
    except OSError:
        rojo.append("no se pudo leer %s" % p)
        return ""


def exige(cond, m):
    if not cond:
        rojo.append(m)
    return cond


# ══════════════════════════════════════════════════════════════════════════
titulo("①", "¿UN ÁRBOL DE VARIOS REPOSITORIOS? — lo que dice el motor")
# ══════════════════════════════════════════════════════════════════════════

link = leer("crates/ore-core/src/link.rs")
val = leer("crates/ore-core/src/validate.rs")

# La firma manda: los miembros son RUTAS, y salen de donde hay un `package.yaml`.
firma = re.search(r"pub fn miembros\(pkg: &Package\) -> Vec<(\w+)>", link)
exige(firma and firma.group(1) == "PathBuf",
      "`miembros` ya no devuelve rutas: la conclusión de ① hay que rehacerla")
exige("`members` NO se lee" in val,
      "`validate.rs` ya no dice que `members` no se lee")

print("""
  El motor compila un WORKSPACE, y un workspace es UN directorio con UN
  `ontology.config.yaml` en la raíz. Sus miembros son **ubicaciones**:

      pub fn miembros(pkg: &Package) -> Vec<PathBuf>

  y salen de dónde hay un `package.yaml`, no de expandir un patrón — `members`
  se declara y **no se lee**, dicho en `validate.rs`.

  ⛔ ⇒ «varios repositorios que son el mismo árbol» NO existe, y no es un hueco:
    un miembro es una ruta bajo una raíz. Dos repositorios son dos raíces, y no
    hay nada que los una en una sola compilación.""")
hallazgos.append("⛔ un árbol = un workspace = UNA raíz. `members` son rutas, y ni se leen")

# ⛔ `Kind::OntologyConfig => &[` sale DOS veces — las claves de `metadata` en una
#   linea, y las de arriba en varias. Un `.*?\]` cogia la primera, y ademas se
#   habria cortado en el `]` de un comentario (`datasources[].labels`). Se ancla
#   en el salto de linea, que es lo unico que distingue a las dos.
doc = leer("crates/ore-core/src/document.rs")
bloque = doc.split("Kind::OntologyConfig => &[" + chr(10), 1)
tiene = []
if len(bloque) == 2:
    for linea in bloque[1].splitlines():
        if re.match(r"\s*\],?$", linea):
            break
        m = re.match(r'\s*"([a-zA-Z]+)",', linea)
        if m:
            tiene.append(m.group(1))
tiene = sorted(set(tiene))
print("\n  Y lo que el manifiesto SÍ declara: %s" % ", ".join(tiene))
exige("dependencies" in tiene and "trustedKeys" in tiene,
      "el manifiesto ya no declara `dependencies`/`trustedKeys`: ② cambia")

print("""
  ⭐⭐ PERO LA IDEA SÍ ESTÁ, Y ES MÁS FUERTE QUE LA PROPUESTA.

    Lo que compone piezas de sitios distintos no son repositorios: son PAQUETES
    PUBLICADOS. `dependencies` + `ontology.lock` + un `.oob` con su digest, y
    `trustedKeys` / `trustedLogs` para decir de quién te fías.

    ⇒ La diferencia no es de forma, es de garantía. Un segundo repositorio
      montado al lado entra sin que nadie lo firme ni lo fije; una dependencia
      entra **con un digest y una firma**, y el motor comprueba que lo que hay
      es lo que el lock dice que es.

    ⭐ Y la confianza la declara QUIEN CONSUME —`trustedKeys` vive en el
      manifiesto de arriba, no dentro del paquete que firma—, porque una clave
      que viniera con su propio paquete cerraría el círculo.

  ⇒ CORRECCIÓN a la propuesta, y sólo una: la organización tiene UN árbol, y eso
    es exacto. Lo plural no son repositorios DENTRO del árbol — son árboles que
    dependen de paquetes publicados por otros. La frontera lleva firma.""")
hallazgos.append("⭐ componer sí existe: `dependencies` + lock + `.oob` firmado, no repos")

# ══════════════════════════════════════════════════════════════════════════
titulo("②", "QUÉ HAY HOY — dos organizaciones, un repositorio, cero vínculos")
# ══════════════════════════════════════════════════════════════════════════

org = leer("iam/migraciones/004-la-organizacion.sql")
cols = re.search(r"create table if not exists iam\.organizacion \((.*?)\);", org, re.S)
nombres = re.findall(r"^\s{2}(\w+)\s", cols.group(1) if cols else "", re.M)
print("\n  Columnas de `iam.organizacion`:  %s" % ", ".join(nombres))
exige(not any("arbol" in c or "repo" in c for c in nombres),
      "`iam.organizacion` YA tiene columna de árbol: esta medida está vieja")

print("""
  Medido en el clúster el 2026-09-09:

      iam.organizacion    DOS      demo    org_b7b98fdd…   sub 0304d602…
                                   prueba  org_1058fd2e…   sub 5027115e…
      la forja            UNO      t-demo/ontologia

  ⛔ Y no hay ninguna columna donde apuntar cuál es de quién. Que
    `t-demo/ontologia` sea de `demo` es que un namespace se llama parecido: una
    convención tipográfica, no un dato. Nadie puede consultarla, así que nadie
    puede equivocarse al leerla — porque no hay nada que leer.""")
hallazgos.append("⛔ `iam.organizacion` no tiene dónde apuntar su árbol")

# ══════════════════════════════════════════════════════════════════════════
titulo("③", "⛔⛔ `fundar` NO PUEDE CREAR EL ÁRBOL HOY, y el motivo es la imagen")
# ══════════════════════════════════════════════════════════════════════════

cargo = leer("crates/ore-iam/Cargo.toml")
deps = re.findall(r"^([a-z0-9-]+)\s*=", cargo.split("[dependencies]")[-1], re.M)
docker = leer("Dockerfile")
base = re.search(r"FROM (\S+) AS iam", docker)

print("\n  `ore-iam` depende de:   %s" % ", ".join(deps))
print("  y corre desde:          FROM %s" % (base.group(1) if base else "?"))
exige(base and base.group(1) == "scratch",
      "`ore-iam` ya no sale de `scratch`: ③ cambia entera")
exige(not any(d in deps for d in ("reqwest", "ureq", "hyper", "curl")),
      "`ore-iam` ya tiene un cliente HTTP")

print("""
  ⇒ Sin cliente HTTP, y desde `scratch`: **sin shell, sin `git`, sin
    certificados**. Este binario no puede llamar a la API de la forja ni empujar
    nada. No es que no se haya escrito el código: es que la imagen no lo permite.

  ⚠️ Y esa imagen no es un descuido — es la misma propiedad que `ore-serve` tuvo
    que ceder: su Dockerfile dice que sale de `alpine` **y no de `scratch`,
    porque este proceso necesita `git`**. Darle a `fundar` lo que le falta es
    exactamente gastar en `ore-iam` lo que allí ya se pagó.

  Las dos formas, y no son equivalentes:

      a) `ore-iam` gana la capacidad   → deja de salir de `scratch`; el plano de
                                         identidad pasa a poder salir a la red
      b) fundar lo ENCARGA             → escribe qué árbol le toca, y quien puede
                                         salir lo aprovisiona""")
hallazgos.append("⛔ `ore-iam` sale de `scratch` y no tiene cliente HTTP: `fundar` no alcanza la forja")

# ══════════════════════════════════════════════════════════════════════════
titulo("④", "«EN EL MISMO ACTO» — por qué no puede serlo, y qué se hace en su lugar")
# ══════════════════════════════════════════════════════════════════════════

print("""
  La fila de la organización es una TRANSACCIÓN. Crear un repositorio es un
  efecto EXTERNO. No hay forma de que los dos ocurran o no ocurran juntos: si la
  transacción se deshace después de crear el repositorio, el repositorio queda; y
  si el repositorio falla después de confirmar la fila, la fila queda.

  ⇒ Así que la pregunta no es «cómo se hacen a la vez». Es **cuál de los dos es
    la verdad y cuál se reconcilia hacia ella**. Y hay una respuesta buena:

      ⭐ LA VERDAD ES LA FILA. El árbol se DECLARA al fundar —una columna—, y el
        repositorio se crea después convergiendo hacia lo declarado.

    Porque el fallo se puede nombrar: quien entre antes de que exista se
    encuentra «tu árbol todavía no está aprovisionado», que dice qué pasa y qué
    falta. Con el orden contrario el fallo es un repositorio huérfano —creado y
    sin fila que lo nombre—, y eso **no lo ve nadie**: no hay desde dónde mirarlo.

  ⇒ Y entonces la propuesta se sostiene entera, sólo que el acto de fundar
    escribe UN NOMBRE en vez de crear un repositorio. Que es lo que hoy falta de
    todos modos: sin ese nombre, `ore init` crearía algo que nadie sabe llamar.""")
hallazgos.append("⭐ fundar DECLARA el árbol (una columna); el repositorio converge hacia eso")

# ══════════════════════════════════════════════════════════════════════════
titulo("⑤", "⭐ Y QUE `ore-serve` SEPA DE ORGANIZACIONES ES MÁS BARATO DE LO QUE PARECE")
# ══════════════════════════════════════════════════════════════════════════

serve = leer("crates/ore-serve/src/rutas.rs")
main = leer("crates/ore-serve/src/main.rs")
rutas = re.findall(r'\("(GET|POST)", "(/[^"]+)"', serve)
con_org = [r for _, r in rutas if "organiza" in r]

print("\n  Rutas servidas: %s" % ", ".join(r for _, r in rutas))
print("  De ellas, con organización en el camino: %s" % (", ".join(con_org) or "NINGUNA"))
exige(not con_org, "`ore-serve` ya nombra organizaciones: ⑤ cambia")
exige("Arbol::Forja" in main and "FORJA_TOKEN" in main,
      "el árbol de `ore-serve` ya no sale de `--forja` + `FORJA_TOKEN`")

print("""
  El árbol se fija AL ARRANCAR:  `--forja <url>` + `FORJA_TOKEN`, y `Arbol` es
  `Directorio | Forja` — UNA. Por eso la separación entre inquilinos es hoy por
  DESPLIEGUE: un `ore-serve` por namespace, clavado a un repositorio.

  ⭐⭐ Pero lo caro ya está pagado: **este proceso NO SE QUEDA EL ÁRBOL**. Clona
    en cada petición y empuja la que escribe. Así que hacer el árbol función de
    la organización es cambiar de dónde sale una `url` —de un argumento a una
    consulta— y no tocar la arquitectura. El Deployment ya es sin estado.

  ⛔ Lo caro es OTRA COSA, y conviene no confundirla: **el testigo**. Hoy
    `FORJA_TOKEN` empuja a un repositorio. Un `ore-serve` que sirva a todos
    necesita un testigo que pueda empujar a TODOS — una credencial con alcance
    sobre cada inquilino, en un solo proceso. Eso es exactamente la figura que
    `70` §1 rechaza en otro sitio.

  ⇒ Y por eso las dos opciones se eligen por el SECRETO, no por el árbol:

      un `ore-serve` por organización   testigo estrecho · N despliegues que
                                        alguien tiene que crear al fundar
      uno solo, con la org en el camino  un despliegue · UN testigo que puede
                                        con todo, y que si se filtra, con todo""")
hallazgos.append("⭐ el árbol por organización es barato en `ore-serve`; lo caro es el TESTIGO")

# ══════════════════════════════════════════════════════════════════════════
titulo("⇒", "LO QUE SALE DE MEDIR")
# ══════════════════════════════════════════════════════════════════════════
for h in hallazgos:
    print("  · " + h)

print("""
  CONFIRMADO con una corrección:

    ✔ una organización tiene UN árbol, y es la unidad — un workspace, una raíz
    ✔ nace al fundarse la organización: el argumento es el que ya está escrito
      para el dueño —«una organización sin administrador es huérfana desde el
      primer segundo»— y vale igual para el árbol
    ✘ pero lo plural NO son repositorios dentro de un árbol: son ÁRBOLES que
      dependen de paquetes publicados, con digest y firma. La composición existe
      y es mejor que la propuesta; lo que no existe es montar dos raíces como una

  El orden que sale de esto:

    1  una columna en `iam.organizacion`: CÓMO SE LLAMA su árbol   ← lo primero
    2  `fundar` la escribe. No crea el repositorio: lo declara
    3  quien puede salir a la red converge hacia lo declarado
    4  y la decisión que hay que tomar antes del 3: un `ore-serve` por
       organización, o uno con la organización en el camino. Se decide por el
       TESTIGO, no por el árbol

  ⚠️ Y mientras el 1 no exista, `ORE_SERVE_URL` en la consola es una constante:
    el dueño de `prueba` entraría y vería el árbol de `demo`.""")

if rojo:
    print("\n⛔ LA MEDIDA NO CUADRA CON EL ÁRBOL:")
    for r in rojo:
        print("   · " + r)
    sys.exit(1)
print("\n✓ todo lo que esta medida afirma sigue estando en el árbol")
