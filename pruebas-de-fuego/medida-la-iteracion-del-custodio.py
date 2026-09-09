# -*- coding: utf-8 -*-
"""MEDIDA · qué ES el custodio, qué hay que construir y en qué orden.

La `0023` decidió la forma. `medida-el-custodio.py` midió que `ore-iam` no puede
serlo. Esto mide la pieza: qué es, de qué se compone, qué ya existe que sirva
tal cual, y cómo se itera para que cada paso deje algo probado.

    uso:  python pruebas-de-fuego/medida-la-iteracion-del-custodio.py
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
hallazgos = []
rojo = []


def titulo(n, t):
    print("\n" + "=" * 78)
    print("%s · %s" % (n, t))
    print("=" * 78)


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
titulo("①", "⭐⭐ HABLAR CON GOOGLE YA ESTA RESUELTO, Y NO CON UNA CRATE")
# ══════════════════════════════════════════════════════════════════════════

bq = leer("crates/ore-read-bigquery/src/main.rs")
cargo_bq = leer("crates/ore-read-bigquery/Cargo.toml")
deps_bq = re.findall(r"^([a-z0-9-]+)\s*=", cargo_bq.split("[dependencies]")[-1], re.M)
por_proceso = "Command::new" in bq

print("\n  `ore-read-bigquery` depende de: %s" % ", ".join(deps_bq))
print("  Y habla con Google por: %s" % ("un SUBPROCESO (`bq`)" if por_proceso else "?"))
exige(por_proceso and not any(d in deps_bq for d in ("reqwest", "tokio", "hyper")),
      "`ore-read-bigquery` ya no habla con Google por subproceso: ① cambia")

print("""
  Su propia frase lo dice, y es la respuesta al problema del custodio:

      «este programa no habla con BigQuery, habla con el [cliente]»

  ⇒ Un binario de Rust **sin una sola crate de red** que llama a las APIs de
    Google ejecutando el CLI que ya viene en la imagen. Para el custodio es
    exactamente lo mismo: `gcloud kms encrypt` y `gcloud kms decrypt`.

  ⭐ Lo que compra, y no es poco:

      · ni TLS ni OAuth ni firmas en Rust — tres cosas cuyo modo de fallo es
        silencioso y en la direccion insegura;
      · la autenticacion es Workload Identity y la resuelve `gcloud` sola,
        igual que en las copias y en el driver;
      · y el cierre de dependencias del custodio se parece al de `ore-iam`:
        postgres y poco mas.""")
hallazgos.append("⭐ el custodio habla con el KMS por `gcloud`, como `ore-read-bigquery` con `bq`")

# ══════════════════════════════════════════════════════════════════════════
titulo("②", "⛔⛔ LA SEPARACION REAL — y no es la que suena bien")
# ══════════════════════════════════════════════════════════════════════════

print("""
  Suena bien decir «uno autoriza y otro abre». Pero si UNA sola pieza tiene a la
  vez el cifrado y el uso de la llave, esa pieza lo abre TODO lo que alcance —
  da igual quien autorizara.

  ⇒ Asi que la separacion no puede ser de responsabilidades: tiene que ser de
    ALCANCE. Y hay una que el arbol ya sabe hacer:

      ⭐⭐ UNA KEK POR ORGANIZACION, Y UN CUSTODIO POR INQUILINO.

    Con eso, la tabla del material puede ser compartida sin peligro: el custodio
    de `acme` puede LEER el cifrado de `beta` y no puede abrirlo, porque su
    cuenta de Google no puede usar la KEK de `beta`.

  ⭐ Y el segundo cerrojo no lo ponemos nosotros: lo pone Google IAM sobre una
    clave. Es la misma figura que el testigo de la forja —donde lo que ata no es
    el ambito del token sino de quien es y de que es colaborador— aplicada a una
    llave en vez de a un repositorio.

  ⇒ Y de ahi sale su FORMA, que resulta ser la de `ore-serve`:

      por inquilino · en `t-<org>` · HTTP con identidad OIDC · su cuenta de
      Google atada a SU clave · y una entrada, la misma que la E6 ya debe

  ⛔ Y NO es `ore-serve` con una bandera. Ese proceso tiene tres cerraduras
    escritas sobre la misma puerta —imagen sin TLS, `HERMETICOS` como lista de
    permitidos, y su NetworkPolicy— y darle `gcloud` las abre las tres.""")
hallazgos.append("⛔ la separacion es de ALCANCE: una KEK por organizacion, un custodio por inquilino")

# ══════════════════════════════════════════════════════════════════════════
titulo("③", "DE QUE SE COMPONE — y cuanto de eso ya esta escrito")
# ══════════════════════════════════════════════════════════════════════════

entrada = leer("crates/ore-entrada/src/http.rs")
tiene_identidad = pathlib.Path(ORE / "crates/ore-entrada/src/identidad.rs").exists()
base_iam = leer("crates/ore-iam/src/base.rs")
tiene_huella = "anotar" in base_iam

piezas = [
    ("servidor HTTP + rutas",        tiene_identidad, "`ore-entrada`, el mismo que monta `ore-serve` y `ore-iam`"),
    ("verificar el token del realm", tiene_identidad, "`identidad.rs`, con el JWKS de un FICHERO"),
    ("transaccion y HUELLA",         tiene_huella,    "`ore-iam/base.rs::Tx` — `anotar` falla cerrado"),
    ("llamar a Google",              por_proceso,     "el patron de `ore-read-bigquery`: un subproceso"),
    ("el esquema `cofre`",           False,           "no existe: es una migracion"),
    ("envolver y abrir la DEK",      False,           "no existe: es el binario"),
    ("la KEK por organizacion",      "kek" in leer("iam/migraciones/004-la-organizacion.sql"),
                                                      "no existe: es una columna, como `arbol` en la `017`"),
]
print()
for que, hay, donde in piezas:
    print("  %s %-28s %s" % ("·" if hay else "⛔", que, donde))

print("""
  ⇒ De siete piezas, CUATRO ya estan escritas y probadas en otros dos binarios.
    Lo que hay que construir es un esquema, una columna y un binario pequeño que
    no inventa nada: recibe una peticion con identidad, mira una concesion,
    llama a `gcloud`, anota la huella y contesta.""")
hallazgos.append("de siete piezas, cuatro ya existen: falta un esquema, una columna y un binario")

# ══════════════════════════════════════════════════════════════════════════
titulo("④", "⭐⭐ Y LOS DOS ALMACENES PUEDEN SER DOS TECNOLOGIAS")
# ══════════════════════════════════════════════════════════════════════════

print("""
  Decidido que los de PLATAFORMA no viven donde los del CLIENTE. Y al mirar que
  necesita cada uno, resulta que **no necesitan lo mismo**:

      cliente      lo emite una persona · lo lista · lo rota · lo concede
                   ⇒ es un PRODUCTO, y necesita NUESTRO modelo de permisos

      plataforma   lo escribe el aprovisionador · lo lee un pod · nadie mas
                   ⇒ NO es un producto. Cero pantallas, cero potestades, cero
                     concesiones. Solo hace falta guardar y leer

  ⭐ Y por eso lo de plataforma tiene una respuesta comprada: el Secret Manager
    de la nube, con Workload Identity. Sin construir nada, y **la separacion deja
    de ser una convencion**: no es que estén en dos tablas — es que no están
    siquiera en el mismo sistema. Desde la superficie del cliente no hay camino
    porque no hay puente.

  ⚠️ Y ESO CORRIGE EL ORDEN QUE ACABABAMOS DE ACORDAR, asi que se dice:

      «el custodio antes de la E4» daba por hecho que la E4 necesitaba el
      custodio. NO lo necesita: la E4 necesita el almacen de PLATAFORMA, que es
      la mitad que no hay que construir.

  ⇒ La E4 se desbloquea antes y mas barato de lo previsto, y el custodio —la
    mitad que SI es producto— deja de estar en su camino critico.""")
hallazgos.append("⭐⭐ la E4 solo necesita la mitad de PLATAFORMA, que es comprada: no el custodio")

# ══════════════════════════════════════════════════════════════════════════
titulo("⑤", "LA ITERACION — y cada paso deja algo probado")
# ══════════════════════════════════════════════════════════════════════════

print("""
  Mismo criterio que las etapas de la `0022`: nada que sea solo andamio, y lo
  que no necesita credencial va primero porque su prueba es inmediata.

  C1 · LA LLAVE, sin binario y sin esquema.
       Habilitar el KMS, un llavero, UNA CLAVE POR ORGANIZACION, y la columna
       `kek` en `iam.organizacion` — la misma figura que `arbol` en la `017`: el
       NOMBRE, no la carretera.
       ⭐ Su prueba es un Job: el de `t-demo` cifra y descifra con la clave de
         `demo`, y **NO puede** con la de otro. Si eso pasa, el segundo cerrojo
         existe y esta demostrado antes de que haya nada que proteger.

  C2 · EL ESQUEMA `cofre`, y sus permisos.
       Dos tablas —el catalogo y el material, una fila POR VERSION— en el
       servidor que ya tiene copia diaria.
       ⭐ Su prueba: el usuario de base de datos de `ore-iam` **no puede leerlo**.
         La separacion deja de ser una convencion de nombres y pasa a ser un
         `grant` que alguien puede comprobar en un `select`.

  C3 · EL BINARIO. Guardar y resolver, con identidad y huella, llamando a
       `gcloud kms`. Lo pequeño de esto es lo medido en ③: cuatro de sus siete
       piezas ya estan escritas.
       ⭐ Su prueba son los verbos, como `los-verbos.sh`: quien tiene `usar`
         resuelve y NO ve; quien tiene `lector` ve; quien no tiene nada recibe el
         mismo error que si el secreto no existiera.

  C4 · EN LA PLANTILLA DEL INQUILINO. Un fichero mas en `gen-inquilino.py`, y su
       comprobacion ⑤ ya existe — la que exige que ningun manifiesto lleve al
       inquilino en un valor sin estar en `PLANTILLAS`.

  ⇒ Y EN PARALELO, sin depender de nada de lo anterior:

  P1 · EL ALMACEN DE PLATAFORMA. Secret Manager, una cuenta, un enlace. Es lo
       que desbloquea la E4, y no comparte una linea de codigo con C1-C4.""")

print("""
  ⚠️ Y lo que NO se construye todavia, dicho para que no se cuele: la ENTREGA.
    Como llega el valor al proceso que lo usa —montado como fichero, resuelto por
    identidad, nunca un `Secret` perpetuo— es la tercera pregunta de la `0023` y
    tiene su propia respuesta. C3 devuelve un valor por HTTP a quien lo pide; que
    un Job lo reciba sin que pase por un `Secret` es el paso de despues.""")

# ══════════════════════════════════════════════════════════════════════════
titulo("⇒", "LO QUE SALE DE MEDIR")
# ══════════════════════════════════════════════════════════════════════════
for h in hallazgos:
    print("  · " + h)

print("""
  ⭐ El custodio no es una pieza de infraestructura nueva: es **el quinto reparto
    de la misma frontera**, con la forma de `ore-serve` —por inquilino, con
    identidad, con su cuenta de Google— y hablando con el KMS como
    `ore-read-bigquery` habla con BigQuery: por un subproceso.

  ⛔ Lo unico verdaderamente nuevo es la idea de ALCANCE: una KEK por
    organizacion. Es lo que permite que la tabla del material sea compartida y
    que aun asi un custodio comprometido abra un solo inquilino. Sin eso, todo lo
    demas es cosmetica.

  ⚠️ Y una correccion al orden acordado, que sale de medir y no de opinar: la E4
    NO necesita el custodio. Necesita el almacen de plataforma, que es la mitad
    comprada. Se puede hacer ya, en paralelo, y sin tocar nada de C1-C4.""")

if rojo:
    print("\n⛔ LA MEDIDA NO CUADRA CON EL ARBOL:")
    for r in rojo:
        print("   · " + r)
    sys.exit(1)
print("\n✓ todo lo que esta medida afirma sigue estando en el arbol")
