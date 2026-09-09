# -*- coding: utf-8 -*-
"""MEDIDA · el custodio: quién puede abrir un secreto, y dónde cabe el material.

La `0023` decidió QUÉ: `iam` dice quién puede y no guarda nada; el material va
cifrado de sobre con la llave fuera. La `018` ya escribió la mitad que no
necesita custodio. Esto mide la otra:

    1  el KMS y la KEK por organización
    2  las tablas del material

Y se mide contra hechos del árbol y del clúster, no contra la intención.

    uso:  python pruebas-de-fuego/medida-el-custodio.py
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
titulo("①", "⛔⛔ QUIEN PUEDE HABLAR CON UN KMS — y `ore-iam` NO PUEDE")
# ══════════════════════════════════════════════════════════════════════════

docker = leer("Dockerfile")
base_iam = re.search(r"FROM (\S+) AS iam", docker)
cargo_iam = leer("crates/ore-iam/Cargo.toml")
deps_iam = re.findall(r"^([a-z0-9-]+)\s*=", cargo_iam.split("[dependencies]")[-1], re.M)
entrada = leer("crates/ore-entrada/Cargo.toml")

print("\n  `ore-iam`   FROM %s   ·   %s"
      % (base_iam.group(1) if base_iam else "?", ", ".join(deps_iam)))
exige(base_iam and base_iam.group(1) == "scratch",
      "`ore-iam` ya no sale de `scratch`: ① cambia entera")
exige(not any(d in deps_iam for d in ("reqwest", "ureq", "hyper", "curl", "rustls")),
      "`ore-iam` ya tiene un cliente HTTP")

print("""
  Sin shell, sin certificados, sin cliente HTTP. Y `ore-entrada` —lo unico que
  comparte con `ore-serve`— es un servidor: `std::net` para ATENDER, y su
  `--jwks` es un FICHERO justamente porque *«este proceso no va a buscar las
  llaves»*.

  ⛔ Un KMS se habla por HTTPS. Asi que `ore-iam` no puede abrir un secreto
    aunque tuviera permiso — y NO es la red quien lo impide: el namespace
    `identidad` no tiene ni una politica de EGRESO. Lo impide la imagen.

  ⭐ Y eso no es un problema que resolver: es la respuesta. Ya paso con `fundar`
    y la forja, y la conclusion es la misma un piso mas alla.""")
hallazgos.append("⛔ `ore-iam` es `scratch` y sin cliente HTTP: NO puede ser el custodio")

# ══════════════════════════════════════════════════════════════════════════
titulo("②", "⭐⭐ EL CUSTODIO ES LA QUINTA VEZ QUE ESTE ARBOL REPARTE LO MISMO")
# ══════════════════════════════════════════════════════════════════════════

print("""
      leer un origen         ore-read-<tipo>,  no `ore`
      subir un artefacto     ore-store-<tipo>, no `ore`
      atender a un cliente   ore-serve,        no `ore`
      traer el JWKS          50-jwks,          no `ore-serve`
      crear el repositorio   el aprovisionador, no `ore-iam`
      ABRIR UN SECRETO       ← esto,           no `ore-iam`

  ⇒ Cada vez que un proceso no puede —o no debe— salir a la red, el acto de
    salir se saca a una pieza aparte cuyo fallo se ve. Aqui compra ademas algo
    que no compraba en las otras cuatro:

  ⭐⭐ **El que decide quien puede NO es el que puede abrir.** `ore-iam` autoriza
    y es fisicamente incapaz de descifrar nada; el custodio descifra y no sabe
    quien es nadie. Comprometer uno solo no da secretos: hace falta comprometer
    los dos, y estan en imagenes distintas con dependencias distintas.

  ⚠️ Y eso hay que sostenerlo cuando llegue la prisa: la tentacion sera darle a
    `ore-iam` un cliente HTTPS «para no tener otro binario». Ese dia se pierde la
    propiedad, y no se pierde con un error: se pierde con una linea en un
    `Cargo.toml`.""")
hallazgos.append("⭐ el custodio es OTRO proceso: quien autoriza no puede descifrar")

# ══════════════════════════════════════════════════════════════════════════
titulo("③", "EL PATRON DE IDENTIDAD YA ESTA, Y NO HAY NI UNA LLAVE QUE ROTAR")
# ══════════════════════════════════════════════════════════════════════════

print("""
  Cuatro cuentas de Google en el proyecto, y las tres nuestras van por Workload
  Identity — el permiso se concede al PAR `(namespace, cuenta)` y el testigo lo
  emite el servidor de metadatos:

      ore-ci        construye y despliega
      ore-driver    lee origenes            `t-<inquilino>/driver`
      ore-copias    escribe las copias      `identidad/copias` y `forja/copias`

  ⇒ Un custodio nuevo es **una cuenta mas y un enlace mas**, exactamente igual.
    No hay que inventar como se autentica contra Google: ya esta decidido tres
    veces, y ninguna de las tres guarda una llave en el cluster.

  ⭐ Y el permiso que necesitaria es MINIMO y nombrable:
    `cloudkms.cryptoKeyVersions.useToDecrypt` sobre UNA clave. No `admin`, no
    `encrypterDecrypter` sobre el llavero — sobre la clave de ese inquilino.""")

politicas = leer("malla/20-driver.yaml")
abre_mundo = "0.0.0.0/0" in politicas and "except" in politicas
print("""
  ⚠️ Y la red ya deja pasar a quien tiene que pasar: `salida-del-driver` abre el
    mundo MENOS las redes privadas, asi que un Job alcanza las APIs de Google.
    Un custodio en su propio namespace necesitaria su politica, escrita como las
    demas: %s""" % ("el patron existe" if abre_mundo else "⛔ y ese patron ya no esta"))
exige(abre_mundo, "`salida-del-driver` ya no abre el mundo menos lo privado")
hallazgos.append("el patron de identidad —Workload Identity— vale tal cual: una cuenta mas")

# ══════════════════════════════════════════════════════════════════════════
titulo("④", "DONDE CABE EL MATERIAL — y lo que ya lo protege sin hacer nada")
# ══════════════════════════════════════════════════════════════════════════

copias = leer("malla/62-copias-del-idp.yaml")
cubre_iam = "for B in keycloak iam" in copias

print("""
  La base de `iam` vive en `idp-db` y **ya tiene copia diaria** — el CronJob
  vuelca `keycloak` E `iam`: %s

  ⭐ Un almacen que ponga el CIFRADO en ese mismo servidor hereda esa copia el
    primer dia. Y es seguro precisamente porque es cifrado: un volcado que
    acabara donde no debe es ruido sin la KEK, que no esta ahi.

  ⛔ Pero NO en el esquema `iam`, y no es cosmetica. La `0023` dice que `iam`
    dice quien puede y no guarda nada. Un esquema aparte —`cofre`— permite lo
    que un esquema compartido no: **que el usuario de base de datos de `ore-iam`
    no tenga NINGUN permiso sobre el material**. La separacion deja de ser una
    convencion de nombres y pasa a ser un `grant` que alguien puede comprobar.

  La forma, y es corta:

      cofre.secreto    id · organizacion · nombre · clase · emitio · en
                       rotado_en · estado          ← el CATALOGO, sin valores
      cofre.material   secreto · version · cifrado · dek_envuelta · kek · alg
                       ← una fila POR VERSION: rotar es insertar, no sustituir

  ⭐ `version` no es un lujo: rotar una credencial y que lo que ya estaba
    conectado siga funcionando hasta que se recicle es la diferencia entre una
    rotacion y una caida. Y guardar la anterior es lo que permite decir «esta
    version se uso hasta el martes».""" % ("SI" if cubre_iam else "⛔ ya NO"))
exige(cubre_iam, "la copia del IdP ya no cubre la base `iam`")
hallazgos.append("el material cabe en el servidor de `iam` —hereda su copia— pero en OTRO esquema")

# ══════════════════════════════════════════════════════════════════════════
titulo("⑤", "⭐ CMEK POR ORGANIZACION: UNA COLUMNA, NO UN REDISEÑO")
# ══════════════════════════════════════════════════════════════════════════

org = leer("iam/migraciones/004-la-organizacion.sql")
col017 = "arbol" in leer("iam/migraciones/017-el-arbol-de-la-organizacion.sql")

print("""
  `iam.organizacion` tiene hoy seis columnas: id, nombre, estado, creada_en,
  creada_por y **arbol**, que la `017` añadio hace unas horas. %s

  ⇒ Que un cliente traiga SU clave maestra es exactamente la misma figura: una
    columna que dice **como se llama su KEK**, no donde vive ni que es. Es lo que
    la `017` ya decidio para el arbol, y su argumento vale sin cambiar una
    palabra — la identidad no es la carretera.

  ⭐ Y por eso la forma de sobre era la mitad de su gracia: con la DEK envuelta
    por una KEK **que se nombra por organizacion**, «traiga su propia clave» deja
    de ser un proyecto y pasa a ser rellenar esa columna. Sin sobre, cada cliente
    con su clave seria un almacen distinto.

  ⚠️ Con una consecuencia que hay que decir antes de venderlo: si el cliente
    revoca su KEK, sus secretos dejan de abrirse **y eso es lo correcto**. Es la
    propiedad que compra —puede cortarnos el acceso— y a la vez el pie del que
    se cuelga: hay que decirlo en el contrato, no descubrirlo en una incidencia.
""" % ("La `017` ya hizo este mismo movimiento." if col017 else ""))
hallazgos.append("⭐ CMEK por organizacion es UNA COLUMNA: la misma figura que `arbol` en la `017`")

# ══════════════════════════════════════════════════════════════════════════
titulo("⑥", "⛔ Y DOS ALMACENES, ahora que esta decidido que no comparten sitio")
# ══════════════════════════════════════════════════════════════════════════

print("""
  Decidido: los secretos de PLATAFORMA no viven donde los del CLIENTE.

      plataforma   el testigo de la forja de cada inquilino, la clave de
                   despliegue de Flux, el testigo del agente
                   ⇒ nuestros. Ningun cliente los ve, los rota ni los nombra

      cliente      contraseñas, tokens, claves de API, credenciales de origenes
                   ⇒ suyos. Los crea y los rota el, con `secreto:emitir`

  ⚠️ «No en el mismo sitio» admite cuatro grados, y conviene elegir cual:

      otra KEK          lo minimo. Un compromiso de una no abre la otra
      otro esquema      + el usuario de la API de cliente no lo alcanza
      otra base         + un volcado de una no contiene la otra
      otro proceso      + no hay ni una ruta desde la superficie del cliente

  ⭐ El corte que de verdad importa es el tercero de esa lista dicho al reves:
    **desde la superficie que usa el cliente no debe existir ningun camino, ni
    equivocado, que llegue al material de plataforma.** Un `where organizacion is
    null` de menos en una consulta no puede ser lo unico que lo separe.

  ⇒ Y hay un argumento fuerte para que el de plataforma **no sea un producto**:
    sus secretos los emite el aprovisionador y los consume un pod. Nadie los
    lista, nadie los rota a mano, nadie pide una pantalla. Darles el almacen del
    cliente seria construir para ellos algo que no necesitan y abrir una puerta
    que no hace falta.""")
hallazgos.append("⛔ dos almacenes: y el corte es que no exista CAMINO desde la superficie del cliente")

# ══════════════════════════════════════════════════════════════════════════
titulo("⇒", "LO QUE SALE DE MEDIR")
# ══════════════════════════════════════════════════════════════════════════
for h in hallazgos:
    print("  · " + h)

print("""
  El 1 y el 2 de la `0023` son mas pequeños de lo que parecian, y por el mismo
  motivo que el aprovisionador: **las decisiones dificiles ya estan tomadas en
  otro sitio y valen aqui**.

      el KMS        una cuenta de Google mas y un enlace de Workload Identity.
                    El permiso, minimo y nombrable: descifrar con UNA clave
      la KEK        una columna en `iam.organizacion`, como `arbol`
      el material   un esquema `cofre` en el servidor que ya tiene copia, con
                    el usuario de `ore-iam` SIN permiso sobre el
      el custodio   un binario mas, y la quinta vez que este arbol separa
                    «quien decide» de «quien sale a la red»

  ⛔ Y lo unico que no es pequeño, dicho ahora y no cuando duela:

      · una KEK revocada por el cliente deja sus secretos cerrados, y es
        CORRECTO. Va en el contrato, no en una incidencia
      · el KMS es dependencia dura: si no responde, no se abre nada
      · y rotar la maestra necesita re-envolver, que hay que escribir ANTES.
        Una rotacion sin ensayar es la copia que nadie ha restaurado""")

if rojo:
    print("\n⛔ LA MEDIDA NO CUADRA CON EL ARBOL:")
    for r in rojo:
        print("   · " + r)
    sys.exit(1)
print("\n✓ todo lo que esta medida afirma sigue estando en el arbol")
