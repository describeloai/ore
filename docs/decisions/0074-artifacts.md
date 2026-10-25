# 0074 · Artifacts

**Estado:** **borrador · hoja de ruta** (2026-10-25) · AR0 hecho en local. Se trabaja sobre este documento mientras se
construye; al cerrar, se rehace limpio con la imagen del producto. **Decide (propuesta):** que los
paquetes de software que un cliente trae de fuera —un jar de su Artifactory, una rueda de Python de
su equipo— son **activos del catálogo** (`kind: Artifact` en `base.schema.nombre`), que los
resolvers de las capas los sirven a los repositorios como si fueran Central o PyPI, y que a la
larga **todo lo que entra en un Build pasa por aquí**. Sigue a [`0037`](0037-language-servers-in-code-repositories.md)
③c (la capa de la JVM), [`0050`](0050-functions-in-code-repositories.md) (las capas de Python y Node),
[`0038`](0038-assets-catalog-namespaces.md) (`base.schema.nombre`), [`0041`](0041-model-registry.md)
(un registro que es el catálogo) y [`0047`](0047-ore-access-control.md) (quién puede).

## 1 · El problema

El cliente que escribe transforms en Java (y buena parte del de Python) lo hace por **su** código:
librerías internas, parsers de su sector, el motor de reglas de hace diez años. Viven en su
servidor privado (Artifactory, Nexus, GitHub Packages, CodeArtifact) y **no están en Central ni en
PyPI**. Hoy, si su `pom.xml` las pide, la capa falla: no las encuentra.

Conectar nuestros Jobs a su servidor no es la respuesta: pide red hacia su casa, credenciales suyas
dentro de nuestros Jobs y que su servidor esté de pie cada vez que corre un Build. Lo profesional
(Foundry lo hace con Artifact Repositories) es **un puente**: el cliente publica en nosotros desde
donde sí tiene acceso, y nuestros Builds sólo leen de nosotros.

## 2 · Cómo está hoy (medido en el código, 2026-10-25)

| | resuelve con | contra | quién sale a internet |
|---|---|---|---|
| **Java** | Maven (`puesto/jvm/capa/resolver.sh`) | Maven Central | el contenedor `resolver` del Job `55-la-capa-jvm`, y sólo él |
| **Python** | uv (`puesto/python/capa/resolver.sh`) | PyPI | el `resolver` del Job `52-la-capa` |
| **Node** | npm (`puesto/node/capa/resolver.sh`) | el registro de npm | el `resolver` del Job `57-la-capa-node` |

- **El puesto (sesión, Build, Preview) no tiene red.** Baja la capa ya resuelta del almacén al
  arrancar.
- **El resolver sí sale, y a propósito sin nada más**: sin el testigo y sin credencial de nube, para
  que lo que baja de internet no pueda publicar. Lo que resuelve queda con sumas y lock en el
  informe de la capa.
- Lo que la imagen ya trae (`provisto.txt`, `jars.txt`) entra como «provisto»; un choque de
  versiones se avisa.

## 3 · La idea

### 3.1 · Un `Artifact` es un activo del catálogo

- **Un activo = un paquete**, con todas sus versiones: `banco.riesgo.motor_riesgo`, de formato
  `maven`, coordenada `com.banco:motor-riesgo`. El nombre en el catálogo es de ORE; la coordenada es
  del ecosistema y va dentro del documento.
- **Lo hereda todo del catálogo**, como `Model` (0041): ruta única, dueño (0052), permisos (0047),
  búsqueda, linaje y página propia en Assets.
- **El documento vive en el árbol y declara; las versiones y sus ficheros viven en el almacén.**
  Como una `Table`: el documento dice qué es (formato, coordenada, dueño) y el contenido es dato.
  Publicar una versión **no es un commit al árbol** —`main` está protegida (0044 B) y un `mvn deploy`
  no puede pasar por una propuesta—. Detalle en §5.1.
- **Diferencia con `Model`:** aquí **sí hay versiones numeradas y son inmutables**. Una `1.0.0`
  publicada no se reescribe nunca (lo exigen Maven y PyPI, y es lo que hace reproducible un Build).
  Se publica la `1.0.1`, o se retira la `1.0.0`.

### 3.2 · Dónde se ve

- **En Code Repositories, en su home: «+ New artifact» a la izquierda de «+ New repository».** Abre
  la creación (formato, ruta, coordenada) y, ya creado, la pestaña Publish con el token y el comando
  `mvn deploy:deploy-file` / `uv publish` listo para copiar.
- **Sin entrada propia en el sidebar de la plataforma.** Quien lo consume es un repositorio de
  código: su sitio natural es al lado. Por ser activo, además aparece en Assets y en la búsqueda.

### 3.3 · Cómo lo consumen los repositorios

- **Una dirección por celda y formato**, la que hablan las herramientas: `…/artifacts/maven/` (layout
  de Maven) y `…/artifacts/pypi/simple/` (PEP 503). La coordenada se traduce al activo; el permiso
  se mira por activo.
- **El resolver de cada capa la usa además de Central/PyPI**, con un token de lectura de quien
  construye. El cliente sólo escribe la coordenada en su `pom.xml` / `pyproject.toml`, como haría con
  Central.
- **⛔ Confusión de dependencias:** si una coordenada existe en Artifacts, **se toma sólo de
  Artifacts**, nunca de Central/PyPI. Si no, alguien publica `com.banco:motor-riesgo:9.9.9` en
  Central y nuestro resolver lo prefiere. Es el ataque clásico, y se cierra en el resolver.

### 3.4 · El espejo y cortar la salida (la meta)

Hoy el resolver sale a Central/PyPI/npm. **El espejo** es que Artifacts haga de proxy: el resolver
pide a Artifacts, y Artifacts baja de Central la primera vez, guarda el fichero con su suma y lo
sirve desde ahí siempre. Con eso:

- **El resolver deja de salir a internet**; sólo Artifacts sale, a una lista cerrada de orígenes.
- **Todo lo que entra en un Build queda registrado** (qué, cuándo, con qué suma) y puede
  bloquearse.
- Si Central cae o retira un paquete, los Builds siguen.

### 3.5 · Retirar y consumidores

- **Retirar (recall)** una versión: se marca en el documento como retirada, con motivo (p. ej. una
  vulnerabilidad). Los resolvers se niegan a usarla y el informe de la capa dice por qué; los Builds
  que la llevan fijada fallan con ese mensaje hasta que suban de versión. **No se borra**: lo
  construido con ella sigue siendo auditable.
- **Consumidores:** qué repositorios (y con ello qué transforms y datasets) usan cada versión. Sale
  del informe de la capa, que ya guarda qué resolvió cada repositorio, y se escribe como linaje del
  catálogo (`usa` / `usado_por`), igual que una `Function` con un `Model`. Responde a «¿a quién
  afecta retirar `motor-riesgo 3.2.0`?» antes de retirarla.

## 4 · Decisiones (propuestas, por cerrar)

| | propuesta | estado |
|---|---|---|
| **D-A1** | un `Artifact` es un activo del catálogo en `base.schema.nombre`; un activo = un paquete | propuesta |
| **D-A2** | versiones numeradas e inmutables; nunca se reescribe ni se borra una publicada, se retira | propuesta |
| **D-A3** | el documento declara en el árbol; versiones y ficheros en el almacén del inquilino, fuera del árbol (§5.1) | propuesta |
| **D-A4** | formatos: `maven` y `pypi` primero; `npm` después; ni Conda ni Docker | propuesta |
| **D-A5** | una dirección por celda y formato; permiso por activo (0047); publicar con token de alcance a un activo | propuesta |
| **D-A6** | una coordenada de Artifacts nunca se resuelve de fuera (confusión de dependencias) | propuesta |
| **D-A7** | en la consola, «+ New artifact» en el home de Code Repositories; sin entrada en el sidebar | propuesta |
| **D-A8** | el espejo: el resolver sólo habla con Artifacts; sólo Artifacts sale, a orígenes cerrados | propuesta |
| **D-A9** | el token de publicar se firma tras preguntar a `ore-iam` al emitir; vive ≤ 1 h; quitar el permiso no corta uno emitido (excepción acotada a 0047) | propuesta (AR1) |
| **D-A10** | sin `-SNAPSHOT`: una versión que cambia rompe la inmutabilidad y la reproducibilidad de un Build | propuesta (AR1) |
| **D-A11** | se publica sólo en una coordenada que un `Artifact` del árbol declara: primero el activo, luego las versiones | propuesta (AR1) |
| **D-A12** | en AR1, leer es de toda la celda (cualquier repositorio resuelve cualquier artefacto); por activo, con A8 de 0047 | propuesta (AR1) |

## 5 · Hoja de ruta

| paso | qué | listo cuando |
|---|---|---|
| **AR0** | previo: el `kind: Artifact` en la spec (versión nueva de OOS), dónde va el binario, el token de publicar en 0047 | la spec compila un `Artifact` y el plan de rutas está escrito |
| **AR1** | Maven: crear, publicar (`deploy:deploy-file`), servir `…/artifacts/maven/`; la capa de la JVM lo usa con D-A6 | un transform Java en victor importa una clase de un jar publicado en Artifacts |
| **AR2** | PyPI: publicar (`uv publish`/`twine`), servir `…/pypi/simple/`; la capa de Python lo usa | un transform Python importa una rueda publicada |
| **AR3** | consola: «+ New artifact», creación, Publish con instrucciones, página del activo con versiones | se crea y publica desde la consola sin terminal salvo el comando de subir |
| **AR4** | espejo de Central y PyPI; el resolver sin salida a internet | los Jobs de las capas resuelven con la salida cerrada |
| **AR5** | retirar y consumidores (linaje desde el informe de la capa) | retirar una versión dice a quién afecta y los Builds lo explican |
| **AR6** | npm y la capa de Node | — |

### 5.0 · La remodelación: el activo es el `ArtifactRegistry` (esperando el go)

**Por qué.** Comparado con el Artifact Repository de Foundry (2026-10-25): allí el recurso es **un
repositorio que contiene muchos paquetes**, con una URL, y el CI publica cualquier librería con
`mvn deploy`. Nuestro modelo (un activo por paquete, declarado antes, D-A11) tenía un fallo de
fondo: declarar es un commit a `main`, que está protegida, así que **cada librería nueva pediría
una propuesta revisada antes de su primer deploy**, y un CI no puede esperar eso.

**El modelo nuevo, en una frase:** el activo de catálogo es el **registro** (`kind:
ArtifactRegistry`); un registro contiene muchos **paquetes** (una librería: `com.banco:motor-riesgo`)
y cada paquete muchas **versiones**. El registro se declara una vez en el árbol; paquetes y
versiones son dato y nacen al publicar.

```
banco.riesgo.libs                      ← el activo (ArtifactRegistry), en el árbol, con dueño
 ├─ maven  com.banco:motor-riesgo      ← un paquete: nace en su primer deploy (dato)
 │          ├─ 1.0.0  jar, pom          ← versiones y ficheros, inmutables (dato)
 │          └─ 1.1.0
 ├─ maven  com.banco:swift-parser
 └─ pypi   banco-riesgo
            └─ 1.0.0  .whl, .tar.gz
```

**Lo que cambia, pieza a pieza** (AR0 no está empujado: se rehace en su sitio, sin migrar nada)

| pieza | antes (AR0) | ahora |
|---|---|---|
| **la gramática** (v1alpha34) | `kind: Artifact` con `format` y `coordinate` | **`kind: ArtifactRegistry`** con `spec.formats: [maven, pypi]` (lista no vacía, sin repetidos, vocabulario cerrado) y `owner`. Sin coordenada |
| **los códigos** | `OOS1004` (forma) y `OOS2055` (coordenada repetida en el árbol) | `OOS1004` (formatos fuera del vocabulario, vacíos o ausentes), `OOS2003` (un formato repetido). **`OOS2055` desaparece**: la coordenada ya no está en el árbol |
| **la unicidad de la coordenada** | regla del árbol | **regla de la plataforma**: una coordenada es del registro donde se publicó primero; publicarla en otro registro de la celda es 409. Se reclama con `subir` («sube si no estaba») en `ore/artefactos/coordenadas/<formato>/<coordenada normalizada>.json` → `{registro}`: atómico, sin carrera |
| **el núcleo** | `Kind::Artifact`, `coordenada_valida`, `coordenada_normalizada`, `artefactos.rs` | `Kind::ArtifactRegistry` y su regla de forma; **`artefactos.rs` se va**; validar y normalizar coordenadas pasa a `ore-serve` (AR1), que es quien las recibe |
| **el índice de assets** | `artifact:<base>.<schema>.<n>`, `detalle: {format, coordinate}` | `artifactregistry:<base>.<schema>.<n>`, `detalle: {formats}`; carpeta de costumbre `registries/`. Los paquetes y su número los añade `ore-serve` (como el puntero de un dataset) |
| **los bytes** | `ore/artefactos/<base>/<schema>/<n>/<versión>/<fichero>.json` | `ore/artefactos/<base>/<schema>/<registro>/<formato>/<paquete>/<versión>/<fichero>.json`; los bytes, igual, por suma |
| **publicar** (D-A11) | sólo en una coordenada declarada | **en cualquier coordenada nueva**, si el registro admite el formato y nadie la reclamó antes; el paquete nace con el primer fichero |
| **la URL** | una por celda y formato | **una por registro y formato para publicar**: `…/artifacts/<base>/<schema>/<registro>/maven`. **Para leer, una por celda**: `…/artifacts/maven`, que encuentra el registro por la coordenada reclamada. La capa sigue con una sola URL |
| **el token** | por activo (paquete) | **por registro** `{persona, registro, verbo, caduca}`; lo pide el dueño del registro o quien tenga `artefacto:publicar` |
| **las instrucciones** | `deploy:deploy-file` | **las dos**: `pom.xml` con `<distributionManagement>` + `settings.xml` + `mvn deploy` (el CI de un equipo), y `deploy:deploy-file` (un jar suelto traído de su servidor). En PyPI, `uv publish` y `twine` |
| **la consola** (AR3) | «+ New artifact» | **«+ New artifact registry»** junto a «+ New repository»; la página del registro con **Publish** (formato, token, instrucciones) y **Overview** (paquetes, versiones, quién y cuándo) |
| **retirar y consumidores** (AR5) | por versión del activo | por versión **de un paquete del registro**; los consumidores, por paquete. Sin cambio de idea |
| **los permisos de lectura** (D-A12) | toda la celda | toda la celda en AR1; por registro con A8 |

**Lo que no cambia:** los bytes por suma con prefijo propio; la inmutabilidad por fichero; sin
`-SNAPSHOT`; la confusión de dependencias en la capa (una coordenada reclamada en la celda no se
busca fuera: ahora se mira la reclamación); el espejo (AR4); los previos P1–P4.

**Decisiones que se reescriben:** D-A1 (el activo es el registro, un registro = muchos paquetes),
D-A5 (URL por registro para publicar, por celda para leer), D-A6 (la reclamación de la coordenada
decide), D-A11 (se publica en cualquier coordenada nueva de un formato admitido). **Nueva, D-A13:**
una coordenada pertenece a un solo registro de la celda, el primero que la publica; moverla es
retirar y publicar en el otro (AR5).

**Cómo se hace (R1–R3, en local con commit):**

1. **R1 · la spec** — v1alpha34 rehecha en `C:\oos` (un commit nuevo encima de `f58af71`): `01-artifact-registry.md`,
   `artifact-registry.schema.json`, `OOS2055` fuera de `99-errors`, los casos rehechos (válidos: un
   registro maven, uno con dos formatos; inválidos: formato desconocido, `formats` vacío o ausente,
   un formato repetido, `coordinate` como clave, `labels`, en v1alpha33). Bump del submódulo.
2. **R2 · el núcleo** — `Kind::ArtifactRegistry`, su regla de forma, `Code::Oos2055` y
   `artefactos.rs` fuera, el índice y la carpeta; v1alpha34 en verde.
3. **R3 · el documento** — este ADR reescrito como imagen limpia con el modelo nuevo (§3, §4,
   AR0, AR1), sin la historia del cambio más que en una línea.

Después, AR1 sigue su plan con estos cambios (rutas por registro, la reclamación, el token por
registro, las dos instrucciones).

### 5.1 · AR0 · el plan (esperando el go)

AR0 no sirve nada todavía: fija las tres piezas de las que cuelga todo lo demás —la gramática, dónde
viven los bytes y cómo se publica— y despeja con previos lo que no sabemos de las herramientas.

**AR0·1 · `kind: Artifact` en la spec (OOS v1alpha34, en `C:\oos`, y bump del submódulo)**

```yaml
apiVersion: oos.dev/v1alpha34
kind: Artifact
metadata:
  name: motor_riesgo            # único en su schema, como todo el catálogo
  namespace: banco              # la base (obligatoria)
  schema: riesgo                # opcional: sin él, default
  description: …
  owner: user:ana               # 0052
spec:
  format: maven                 # maven | pypi (npm, en AR6)
  coordinate: com.banco:motor-riesgo   # maven: groupId:artifactId · pypi: el nombre del proyecto
```

- **Sólo declara.** Ni versiones ni sumas en el documento: son dato (AR0·2).
- **Reglas nuevas, con código propio:** la coordenada bien formada para su formato (el alfabeto de
  `Capa.java`/`entorno.rs` para Maven; el nombre normalizado de PEP 503 para PyPI); **una coordenada
  es única en el árbol** —dos `Artifact` con `com.banco:motor-riesgo` serían dos verdades para el
  resolver—; `format` y `coordinate` **no cambian** una vez creado (cambiarlos huérfana lo publicado:
  se crea otro).
- **En ORE:** `Kind::Artifact` en `ore-core` (`document.rs`, `assets.rs`, `pertenencia.rs`,
  `link.rs`), el índice de assets lo lista con su coordenada y la conformidad de v1alpha34 pasa. Es la
  misma huella que tuvo `Model` (02b395db, 9c004b1b).

**AR0·2 · dónde viven los bytes: el almacén del inquilino, con prefijo propio**

```
ore/artefactos/<base>/<schema>/<nombre>/<versión>/<fichero>.json   un fichero publicado: suma, tamaño, quién, cuándo
ore/artefactos/<base>/<schema>/<nombre>/<versión>.retirada.json    AR5: motivo, quién, cuándo
ore/artefactos/sha256/<hex>                                        los bytes (.jar, .pom, .whl, .tar.gz)
```

- **El bucket del inquilino**, cifrado con su clave, como el lago y las capas (`ore/puesto/<capa>/`).
  Nada nuevo que aprovisionar.
- **La inmutabilidad la da el almacén, y la unidad es el fichero** (medido en P1 y P2): `Almacen::subir`
  ya es «sube si no estaba» (`Ok(false)` si estaba). Primero los bytes por suma, después su `.json`;
  republicar un fichero es 409 sin carrera. No la versión: PyPI publica cada fichero de una versión
  en su propia petición (la rueda y luego el sdist) y admite añadir ficheros a una versión, nunca
  reemplazarlos; Maven sube jar, pom y sumas por separado. Una versión es los ficheros que tiene.
- **⛔ Prefijo propio y no `ore/v2/blobs/`:** la recogida de blobs (0046 E8·3b) borra lo que ningún
  manifiesto nombra, y un jar publicado no lo nombra ningún manifiesto. Compartir el prefijo
  obligaría a enseñar a la recogida qué es un artefacto; separarlo no cuesta nada (la deduplicación
  entre colecciones y artefactos no compra nada real).
- **Listar versiones = listar el prefijo.** `maven-metadata.xml` y el índice simple de PyPI se
  generan al servir, de esa lista; no se guardan.

**AR0·3 · cómo se publica y cómo se lee: dos credenciales estrechas**

Las herramientas sólo saben de HTTP Basic: Maven con `<server>` en `settings.xml`, uv/twine con
`__token__` y una contraseña. Hace falta una credencial que quepa ahí.

- **Token de publicar:** «Generate token» en la consola (y `ore artifacts token`) →
  `POST /artifacts/{base}/{schema}/{nombre}/token`. `ore-serve` lo **firma** (clave en el cofre) con
  `{quién, activo, verbo: publicar, caduca}`; **no se guarda**. Por defecto 1 h, como el de Foundry.
- **⚠️ Corregido al planear AR1:** `ore-iam` sólo decide con el token del realm de la persona
  (`Ore-Sujeto`, 0047), y un `mvn deploy` no lo trae: trae nuestro token. Así que el permiso **se
  pregunta al emitir**, con el token de verdad, y el token firmado lo lleva hasta que caduca. Es una
  excepción acotada a la revocación en ≤ 30 s de 0047: quitarle el permiso a alguien no corta un
  token ya emitido, que muere solo en ≤ 1 h (D-A9).
- **Token de leer, para el Job de la capa:** `ore-serve`, al lanzar el Job, firma uno de lectura
  `{repositorio, activos que su declaración nombra, caduca con el Job}` y lo monta **sólo** en el
  contenedor `resolver`. Ese contenedor sigue sin testigo ni nube: puede bajar lo publicado y nada
  más.
- **Deuda declarada:** un token largo para el CI del cliente (días o meses) pediría poder revocarlo
  uno a uno, es decir, guardarlo. No entra en AR0.

**AR0·4 · previos (lo desconocido, antes de AR1)**

Medidos el 2026-10-25 con un repositorio de juguete que apunta cada petición
(`pruebas-de-fuego/artifacts/ar0/`: `juguete.py`, `medir.sh`; Maven 3.9 con Temurin 21, uv y twine
en Python 3.14) y, P3, con un pod en t-victor.

| previo | pregunta | lo medido | lo que decide |
|---|---|---|---|
| **P1** | ¿qué peticiones hace `mvn deploy:deploy-file`? | **10**: `PUT` del jar, su `.sha1` y su `.md5`; del pom (lo genera), `.sha1` y `.md5`; `GET maven-metadata.xml` (404 la primera vez) y su `PUT` con sus dos sumas. Basic **preventivo** en los `PUT`. Un 409 y un 401 llegan al usuario tal cual (`status code: 409`) | `PUT` de jar y pom inmutables (409); las sumas se aceptan y se cotejan con lo recibido; `maven-metadata.xml` se acepta y **se tira** —se genera al leer, de los ficheros que hay—. Ningún paso dice «terminé»: por eso la unidad es el fichero (AR0·2) |
| **P2** | ¿cómo publican `uv publish` y `twine upload`? | **un `POST` multipart por fichero** a la API de subida de PyPI, `:action=file_upload`, con `name`, `version`, `filetype`, `sha256_digest`, `blake2_256_digest` y el fichero; Basic con `__token__`. **El nombre llega sin normalizar, y distinto por fichero** (`Banco_Riesgo` en la rueda, `Banco-Riesgo` en el sdist). Republicar es 409 y uv lo enseña. `uv pip install` lee `GET /simple/<normalizado>/` (exige `Content-Type: text/html`), y `HEAD` + `GET` del fichero | el servidor normaliza (PEP 503) para dar con el activo; coteja `sha256_digest` con lo recibido; el índice simple, con su `Content-Type` |
| **P3** | ¿llega el Job de la capa a `ore-serve`? | **sí**: un pod con `ore.dev/rol=driver` (el del Job) en t-victor da `/salud` → 200 y una ruta sin token → 401 (`salida-al-control` + `entrada-al-control`). **Y sale a internet el pod entero**, no sólo el contenedor `resolver`: Central y PyPI → 200 | AR1 no toca la red. AR4 (cortar la salida) es del pod, no del contenedor |
| **P4** | ¿qué hace Maven con el mismo GAV en ORE y en Central, ORE primero? | **⛔ con los ajustes de serie, Maven ni pregunta a ORE**: desde 3.8.1 bloquea todo repositorio `http://` con el espejo `maven-default-http-blocker`, **en silencio** (cero avisos en el log), y todo sale de Central. Sin el bloqueo: lo que está en los dos sale de **ORE**; lo que sólo está en Central, de Central. Al leer, Maven no manda credenciales hasta que le contestan 401 | `ore-serve` es `http://` dentro de la celda: el `settings.xml` del resolver **anula el bloqueo sólo para el repositorio de ORE**. Leer sin token es 401 con `WWW-Authenticate`. El orden **no basta para D-A6**: una versión que ORE no tenga se busca en Central. En AR1, la capa rechaza una coordenada de Artifacts con una versión que Artifacts no tiene; lo transitivo, con el espejo (AR4) |

**AR0 está listo cuando:** v1alpha34 compila un `Artifact` y su conformidad pasa; `ore assets`
lo lista en victor tras un commit al árbol; AR0·2 y AR0·3 quedan escritos aquí como decisión; y los
cuatro previos están medidos con su resultado en este documento.

**Estado de AR0 (2026-10-25), en local sin empujar:**

- ✅ **La spec**: OOS v1alpha34 (`spec/v1alpha34/00-scope.md`, `01-artifact.md`,
  `schemas/v1alpha34/artifact.schema.json`, `OOS2055` en `99-errors`), 14 casos (3 válidos,
  11 inválidos). Commit `f58af71` en `oos`, y el submódulo apunta a él.
- ✅ **El núcleo**: `ApiVersion::V1Alpha34` y `Code::Oos2055` en `ore-base`; `Kind::Artifact` en
  `ore-core` (forma con `coordenada_valida` y `coordenada_normalizada`, en schemas, del paquete
  donde vive, carpeta `artifacts/`), `artefactos.rs` (`OOS2055`, junto a la identidad, antes del
  enlazado) y el índice de assets (`artifact:<base>.<schema>.<nombre>`, con `detalle: {format,
  coordinate}`). v1alpha34 da **14/14**, la conformidad entera 33/33, ore-base y ore-core en
  verde y clippy limpio.
- ✅ **Los cuatro previos**, arriba.
- ⏳ **`ore assets` en victor**: en local ya lista el artefacto; en victor espera a empujar y
  desplegar.
- ⚠️ Lo que se vio al medir: el volumen de `target` compartido entre worktrees (`/src` en los dos)
  daba por fresco un binario de otra copia; AR0 se compiló en un volumen propio
  (`ore-target-q1`).

## 6 · Preguntas abiertas

- **¿Activo = paquete o activo = repositorio** (varios paquetes, como el Artifact Repository de
  Foundry)? Propuesta: paquete, porque permisos, retirada y consumidores son por paquete; el schema
  ya agrupa.
- **¿Publicar desde un repositorio de código de ORE?** (una librería escrita en ORE que otros
  repositorios consumen). Encaja en el mismo activo; no entra en AR1–AR3.
- **¿Cuota de almacén por celda?**
- **¿Escaneo de vulnerabilidades** de lo publicado y de lo espejado? Natural tras AR4.

### 5.2 · AR1 · Maven, el plan (esperando el go)

**El resultado:** alguien de la organización publica su jar con `mvn deploy` contra ORE, y un
transform Java de victor lo importa como si viniera de Central. Seis piezas; las cinco primeras en
local con commit, la sexta es la medida en victor.

**AR1·1 · el almacén de artefactos en `ore-serve`** (`crates/ore-serve/src/artefactos.rs`)

- Escribir un fichero: los bytes a `ore/artefactos/sha256/<hex>` y después
  `ore/artefactos/<base>/<schema>/<nombre>/<versión>/<fichero>.json` con `sha256`, `sha1`, `md5`,
  tamaño, quién y cuándo, con `Almacen::subir` (sube si no estaba → 409 sin carrera). Se calculan
  las tres sumas al recibir: Maven coteja `sha1`/`md5` y lo de dentro es `sha256`.
- Leer: listar versiones y ficheros por prefijo; bajar los bytes por su suma.
- La coordenada → el activo: del árbol compilado (`format`, `coordinate`), por el índice que
  `ore-serve` ya tiene. Sin `Artifact` para la coordenada, 404 al leer y 403 al publicar (D-A11).

**AR1·2 · la puerta Maven** (`/artifacts/maven/…`, en `rutas.rs`)

| petición | respuesta |
|---|---|
| `PUT …/<g>/<a>/<v>/<a>-<v>[-clasificador].<ext>` | 201; 409 si ya estaba; 400 si `<v>` es `-SNAPSHOT` (D-A10) o la ruta no cuadra con la coordenada |
| `PUT ….sha1` / `.md5` / `.sha256` / `.sha512` | 201 si coincide con lo recibido; 400 si no. No se guardan: se calculan |
| `PUT …/maven-metadata.xml` (y sus sumas) | 201 y **se tira**: la metadata se genera (P1) |
| `GET`/`HEAD` de un fichero o de su suma | los bytes; la suma, calculada |
| `GET …/<g>/<a>/maven-metadata.xml` | generada de las versiones que hay (`latest`, `release`, `versions`, `lastUpdated`) |
| cualquiera sin credencial | 401 con `WWW-Authenticate: Basic realm="ore"` (Maven no manda credenciales hasta que se le piden: P4) |

Rutas con `..`, vacías o fuera del alfabeto de la coordenada, 400. Cada publicación deja huella
en la actividad (`artefacto:publicar`, con quién, coordenada, versión y fichero).

**AR1·3 · los tokens** (D-A9, D-A12)

- `POST /artifacts/{base}/{schema}/{nombre}/token`, con el token del realm (la consola, o `curl`):
  puede quien es **dueño** del artefacto (`owner`, 0052) o tiene la potestad
  **`artefacto:publicar`** (migración `iam/migraciones/053`, a ORGADMIN y ACCOUNTADMIN), preguntado a
  `ore-iam`. Devuelve una vez `{usuario: "publicar", token, caduca, settings, comando}`: el
  `<server>` de `settings.xml` y el `mvn deploy:deploy-file` listos para copiar.
- El token: `ore_art_<carga>.<hmac>`, la carga `{persona, activo, verbo, celda, caduca}`. Firma
  HMAC-SHA256 con `ORE_ARTEFACTOS_SECRETO` o, sin él, una clave al azar por proceso (el patrón de
  `teselas.rs`): **reiniciar `ore-serve` invalida los tokens emitidos**, y se dice. No se guarda.
- Al usarlo: firma, caducidad, celda, y que el activo sea el de la coordenada. Un token de publicar
  también lee su activo (el `GET` de la metadata que hace `mvn deploy`).
- **El de leer, para la capa:** `verbo: leer`, activo `*` (toda la celda, D-A12), 20 min. Lo firma
  `ore-serve` al rendir el Job de la capa de la JVM y va en un hueco nuevo de la plantilla,
  `ORE_ARTEFACTOS_TOKEN`. **Se ve en el Job** (`kubectl get job`): por eso sólo lee y caduca pronto.
  `ore-serve` tolera una plantilla sin el hueco (sin él, la capa no mira Artifacts): **binario antes
  que malla**.

**AR1·4 · el `Artifact` se puede crear** (`documentos.rs`)

- `Artifact` en el motor de documentos (carpeta `artifacts/`), para crearlo desde la consola o la
  API antes de AR3. **`format` y `coordinate` no cambian** si ya hay algo publicado: 409 con lo que
  hay. Borrar un artefacto con versiones, 409 (retirar es AR5).

**AR1·5 · la capa de la JVM lo usa** (`puesto/jvm/capa/`, imagen `capa-jvm`, `malla/55`)

- `resolver.sh`, con `ORE_ARTEFACTOS_TOKEN`: escribe un `settings.xml` con el `<server>` de ORE y el
  espejo `maven-default-http-blocker` anulado (`mirrorOf` que no casa con nada), porque `ore-serve`
  es `http://` dentro de la celda (P4). `Capa.java pom` pone el repositorio de ORE **antes** de
  Central.
- **D-A6 antes del espejo:** antes de resolver, por cada dependencia declarada, `GET` de su
  `maven-metadata.xml` en ORE. Si existe y no tiene la versión pedida, el informe dice
  `estado: error` con «`com.banco:motor-riesgo:2.0.0` no está publicada en Artifacts; una coordenada
  de Artifacts no se busca fuera». Lo transitivo queda para AR4, y se dice.
- La plantilla `55` gana el hueco `ORE_ARTEFACTOS_TOKEN` y se empuja a la cola de victor con
  `malla/empujar-plantilla.py` (añadiendo `plantilla-capa-jvm.txt` a lo que empuja), **después** de
  que el binario esté desplegado.

**AR1·6 · la medida en victor** (`pruebas-de-fuego/artifacts/ar1.sh`)

1. Crear `banco.riesgo.motor_riesgo` (`maven`, `com.banco:motor-riesgo`) por la API.
2. Pedir el token y publicar un jar con una clase desde un Docker local, por la API pública (el
   túnel, `https`: sin el bloqueo de P4).
3. Republicar la misma versión → 409. Un token caducado o de otro activo → 401/403. `-SNAPSHOT` →
   400.
4. Un repositorio `transforms-java` de victor declara `com.banco:motor-riesgo:1.0.0`; la capa lo
   resuelve desde ORE (el informe lo dice); un Build importa la clase y escribe su dataset.
5. Declarar `2.0.0` (no publicada) → el informe de la capa da el error de D-A6.

**AR1 está listo cuando** los cinco puntos de AR1·6 pasan en victor. Commits en local; empujar,
cuando lo digas, en este orden: binario (`ore-serve`, `capa-jvm`) y migración `053` → plantilla.

