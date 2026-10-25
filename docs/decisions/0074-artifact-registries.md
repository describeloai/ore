# 0074 · Artifact Registries

**Estado:** **borrador · hoja de ruta** (2026-10-25) · AR0 hecho en local; AR1 planeado. Se trabaja
sobre este documento mientras se construye y se rehace limpio al cerrar. **Decide (propuesta):** que
las librerías que una organización trae de fuera —un jar de su Artifactory, una distribución de
Python de su equipo— viven en un **registro de artefactos**, un activo del catálogo (`kind:
ArtifactRegistry` en `base.schema.nombre`) que contiene muchas librerías; que las capas las
sirven a los repositorios como si vinieran de Central o PyPI; y que a la larga **todo lo que entra
en un Build pasa por aquí**. Sigue a [`0037`](0037-language-servers-in-code-repositories.md) ③c (la
capa de la JVM), [`0050`](0050-functions-in-code-repositories.md) (las capas de Python y Node),
[`0038`](0038-assets-catalog-namespaces.md) (`base.schema.nombre`),
[`0041`](0041-model-registry.md) (un registro que es el catálogo) y
[`0047`](0047-ore-access-control.md) (quién puede). Gramática: [OOS
v1alpha34](../../vendor/oos/spec/v1alpha34/00-scope.md).

## 1 · El problema

El cliente que escribe transforms en Java (y buena parte del de Python) lo hace por **su** código:
librerías internas, parsers de su sector, el motor de reglas de hace diez años. Viven en su servidor
privado (Artifactory, Nexus, GitHub Packages, CodeArtifact) y **no están en Central ni en PyPI**.
Hoy, si su `pom.xml` las pide, la capa falla: no las encuentra.

Conectar nuestros Jobs a su servidor no es la respuesta: pide red hacia su casa, credenciales suyas
dentro de nuestros Jobs y que su servidor esté de pie cada vez que corre un Build. Lo profesional es
**un puente** (Foundry lo hace con Artifact Repositories): el cliente publica en nosotros desde
donde sí tiene acceso, y nuestros Builds sólo leen de nosotros.

## 2 · Cómo está hoy (medido en el código, 2026-10-25)

| | resuelve con | contra | quién sale a internet |
|---|---|---|---|
| **Java** | Maven (`puesto/jvm/capa/resolver.sh`) | Maven Central | el pod del Job `55-la-capa-jvm` |
| **Python** | uv (`puesto/python/capa/resolver.sh`) | PyPI | el pod del Job `52-la-capa` |
| **Node** | npm (`puesto/node/capa/resolver.sh`) | el registro de npm | el pod del Job `57-la-capa-node` |

- **El puesto (sesión, Build, Preview) no tiene red.** Baja la capa ya resuelta del almacén al
  arrancar.
- **El contenedor que resuelve no tiene testigo ni credencial de nube**, para que lo que baja de
  internet no pueda publicar. Lo resuelto queda con sumas y lock en el informe de la capa. La salida
  a internet, en cambio, es **del pod entero** (`ore.dev/rol=driver`; medido en P3).
- Lo que la imagen ya trae (`provisto.txt`, `jars.txt`) entra como «provisto»; un choque de
  versiones se avisa.

## 3 · El producto

### 3.1 · El registro es el activo; las librerías, su contenido

```
banco.riesgo.libs                      ← el activo (ArtifactRegistry), en el árbol, con dueño
 ├─ maven  com.banco:motor-riesgo      ← una librería: nace en su primer deploy (dato)
 │          ├─ 1.0.0  jar, pom          ← versiones y ficheros, inmutables (dato)
 │          └─ 1.1.0
 ├─ maven  com.banco:swift-parser
 └─ pypi   banco-riesgo
            └─ 1.0.0  .whl, .tar.gz
```

```yaml
apiVersion: oos.dev/v1alpha34
kind: ArtifactRegistry
metadata: { name: libs, namespace: banco, schema: riesgo }
spec:
  owner: team:riesgo
  formats: [maven, pypi]
```

- **El registro se declara una vez en el árbol** y lo hereda todo del catálogo, como `Model`
  (0041): ruta única, dueño (0052), permisos (0047), búsqueda y página en Assets. Su documento dice
  **qué formatos admite** y nada más.
- **Las librerías, sus versiones y sus ficheros son dato**: nacen al publicar, sin tocar el árbol.
  Una librería nueva no espera a una propuesta sobre `main` (protegida, 0044 B): la publica el CI
  de quien la escribe.
- **Inmutables.** Un fichero publicado no se reescribe ni se borra (lo exigen Maven y PyPI, y es lo
  que hace reproducible un Build); una versión se retira.
- **Una coordenada pertenece a un registro**, el primero que la publica en la celda
  (`com.banco:motor-riesgo`). Otro registro no puede publicarla: es lo que permite saber de dónde
  servirla y quién responde de ella.

### 3.2 · Dónde se ve

- **En el home de Code Repositories, «+ New artifact registry» a la izquierda de «+ New
  repository».** Crea el registro (ruta y formatos). Su página tiene dos pestañas:
  - **Publish**: el formato, «Generate token» y las instrucciones listas para copiar —el
    `<distributionManagement>` del `pom.xml`, el `settings.xml` y `mvn deploy` para el CI de un
    equipo; `mvn deploy:deploy-file` para un jar suelto traído de su servidor; `uv publish` y
    `twine` en PyPI—.
  - **Overview**: sus librerías, sus versiones, quién y cuándo.
- **Sin entrada propia en el sidebar de la plataforma.** Quien lo consume es un repositorio de
  código: su sitio natural es al lado. Por ser activo, además aparece en Assets y en la búsqueda.

### 3.3 · Cómo lo consumen los repositorios

- **Se publica en el registro, se lee de la celda.** Publicar: `…/artifacts/<base>/<schema>/<registro>/maven`.
  Leer: `…/artifacts/maven` (y `…/artifacts/pypi/simple/`), una sola dirección que encuentra el
  registro por la coordenada. El cliente escribe la coordenada en su `pom.xml` / `pyproject.toml`,
  como haría con Central.
- **El resolver de cada capa la usa además de Central/PyPI**, y antes.
- **⛔ Confusión de dependencias:** una coordenada que un registro de la celda ha reclamado **se toma
  sólo de ahí**, nunca de Central/PyPI. Si no, alguien publica `com.banco:motor-riesgo:9.9.9` en
  Central y el resolver lo prefiere. El orden de los repositorios no basta (P4): una versión que ORE
  no tiene se buscaría fuera.

### 3.4 · El espejo y cortar la salida (la meta)

**El espejo** es que la celda haga de proxy de Central y PyPI: el resolver pide a ORE, y ORE baja de
fuera la primera vez, guarda el fichero con su suma y lo sirve desde ahí siempre. Con eso:

- **El Job de la capa deja de salir a internet**; sólo sale ORE, a una lista cerrada de orígenes.
- **Todo lo que entra en un Build queda registrado** (qué, cuándo, con qué suma) y puede
  bloquearse; lo transitivo también queda cubierto contra la confusión de dependencias.
- Si Central cae o retira un paquete, los Builds siguen.

### 3.5 · Retirar y consumidores

- **Retirar (recall)** una versión de una librería: se marca como retirada, con motivo (p. ej. una
  vulnerabilidad). Los resolvers se niegan a usarla y el informe de la capa dice por qué; los Builds
  que la llevan fijada fallan con ese mensaje hasta que suban de versión. **No se borra**: lo
  construido con ella sigue siendo auditable.
- **Consumidores:** qué repositorios (y con ello qué transforms y datasets) usan cada versión. Sale
  del informe de la capa, que ya guarda qué resolvió cada repositorio, y se escribe como linaje del
  catálogo (`usa` / `usado_por`). Responde a «¿a quién afecta retirar `motor-riesgo 3.2.0`?» antes de
  retirarla.

## 4 · Decisiones (propuestas, por cerrar)

| | propuesta | paso |
|---|---|---|
| **D-A1** | el activo del catálogo es el **registro** (`ArtifactRegistry`, `base.schema.nombre`); contiene muchas librerías | AR0 |
| **D-A2** | ficheros inmutables; nunca se reescribe ni se borra uno publicado; una versión se retira | AR1 |
| **D-A3** | el registro se declara en el árbol; librerías, versiones y ficheros son dato, en el almacén del inquilino (§5.2) | AR0 |
| **D-A4** | formatos: `maven` y `pypi`; `npm` después; ni Conda ni Docker | AR0 |
| **D-A5** | se publica en la dirección del registro; se lee de una por celda y formato | AR1 |
| **D-A6** | una coordenada reclamada por un registro de la celda nunca se resuelve de fuera | AR1 (directas), AR4 (transitivas) |
| **D-A7** | en la consola, «+ New artifact registry» en el home de Code Repositories; sin entrada en el sidebar | AR3 |
| **D-A8** | el espejo: el Job de la capa sólo habla con ORE; sólo ORE sale, a orígenes cerrados | AR4 |
| **D-A9** | el token de publicar se firma tras preguntar a `ore-iam` al emitir y vive ≤ 1 h; quitar el permiso no corta uno ya emitido (excepción acotada a la revocación en ≤ 30 s de 0047) | AR1 |
| **D-A10** | sin `-SNAPSHOT`: una versión que cambia rompe la inmutabilidad y la reproducibilidad de un Build | AR1 |
| **D-A11** | se publica en cualquier coordenada nueva de un formato que el registro admite; la librería nace con su primer fichero | AR1 |
| **D-A12** | en AR1 leer es de toda la celda (cualquier repositorio resuelve cualquier registro); por registro, con A8 de 0047 | AR1 |
| **D-A13** | una coordenada pertenece a un solo registro de la celda, el primero que la publica; moverla es retirar y publicar en el otro | AR1 |

## 5 · Hoja de ruta

| paso | qué | listo cuando | estado |
|---|---|---|---|
| **AR0** | la gramática (v1alpha34), el núcleo, dónde van los bytes, los tokens, y los previos | la spec compila un `ArtifactRegistry` y los previos están medidos | **hecho en local** |
| **AR1** | Maven: publicar, servir, la capa de la JVM lo usa | un transform Java en victor importa una clase de un jar publicado | planeado (§5.3) |
| **AR2** | PyPI: publicar (`uv publish`/`twine`), servir `…/pypi/simple/`; la capa de Python lo usa | un transform Python importa una rueda publicada | — |
| **AR3** | consola: «+ New artifact registry», Publish y Overview | se crea y publica desde la consola sin terminal salvo el comando de subir | — |
| **AR4** | espejo de Central y PyPI; el Job de la capa sin salida a internet | las capas resuelven con la salida cerrada | — |
| **AR5** | retirar y consumidores | retirar una versión dice a quién afecta y los Builds lo explican | — |
| **AR6** | npm y la capa de Node | — | — |

### 5.1 · AR0 · lo hecho (2026-10-25, en local sin empujar)

- **La gramática:** OOS v1alpha34 (`spec/v1alpha34/00-scope.md`, `01-artifact-registry.md`,
  `schemas/v1alpha34/artifact-registry.schema.json`). `formats` obligatoria, no vacía, del
  vocabulario (`OOS1004`), sin repetidos (`OOS2003`); ni `labels` ni claves de librería
  (`OOS1005`). Ningún código nuevo. 9 casos (2 válidos, 7 inválidos).
- **El núcleo:** `ApiVersion::V1Alpha34` en `ore-base`; `Kind::ArtifactRegistry` en `ore-core` (su
  forma, en schemas, del paquete donde vive, carpeta de costumbre `registries/`), el repetido en
  `link::formatos_repetidos`, y el índice de assets (`artifactregistry:<base>.<schema>.<nombre>`
  con `detalle: {formats}`).
- **Los previos P1–P4** (§5.4).
- **Falta:** `ore assets` en victor, que espera a empujar y desplegar.

### 5.2 · Dónde viven los bytes

```
ore/artefactos/<base>/<schema>/<registro>/<formato>/<librería>/<versión>/<fichero>.json   un fichero: sumas, tamaño, quién, cuándo
ore/artefactos/<base>/<schema>/<registro>/<formato>/<librería>/<versión>.retirada.json    AR5: motivo, quién, cuándo
ore/artefactos/coordenadas/<formato>/<coordenada normalizada>.json                        → {registro}: quién la reclamó
ore/artefactos/sha256/<hex>                                                                los bytes (.jar, .pom, .whl, .tar.gz)
```

- **El bucket del inquilino**, cifrado con su clave, como el lago y las capas. Nada nuevo que
  aprovisionar.
- **La inmutabilidad y la reclamación las da el almacén:** `Almacen::subir` es «sube si no estaba».
  Primero los bytes por suma, después el `.json` del fichero: republicar es 409 sin carrera. La
  reclamación de una coordenada es el mismo gesto: el primer registro que la escribe la tiene.
- **La unidad es el fichero, no la versión** (P1, P2): PyPI publica cada fichero de una versión en
  su propia petición y admite añadir ficheros, nunca reemplazarlos; Maven sube jar, pom y sumas por
  separado y ningún paso dice «terminé». Una versión es los ficheros que tiene.
- **⛔ Prefijo propio y no `ore/v2/blobs/`:** la recogida de blobs (0046 E8·3b) borra lo que ningún
  manifiesto nombra, y un jar publicado no lo nombra ningún manifiesto.
- **Listar = listar el prefijo.** `maven-metadata.xml` y el índice simple de PyPI se generan al
  servir; no se guardan.
- **La coordenada se compara como la comparan las herramientas:** tal cual en Maven; normalizada en
  PyPI (PEP 503: minúsculas, y cada tramo de `.`, `_` y `-` vale un `-`). La valida y normaliza
  `ore-serve`, que es quien la recibe.

### 5.3 · AR1 · Maven, el plan (esperando el go)

**El resultado:** alguien de la organización publica su jar con `mvn deploy` en un registro de ORE,
y un transform Java de victor lo importa como si viniera de Central. Cinco piezas en local con
commit y la medida en victor.

**AR1·1 · el almacén de artefactos en `ore-serve`** (`crates/ore-serve/src/artefactos.rs`):
escribir un fichero (§5.2) calculando `sha256`, `sha1` y `md5` al recibir; reclamar la coordenada
en el primer fichero de una librería (si la tiene otro registro, 409, D-A13); listar librerías,
versiones y ficheros por prefijo; bajar los bytes. El registro, del árbol compilado: sin él, 404; si
no admite el formato, 400.

**AR1·2 · la puerta Maven:**

| petición | respuesta |
|---|---|
| `PUT …/<base>/<schema>/<registro>/maven/<g>/<a>/<v>/<a>-<v>[-clasificador].<ext>` | 201; 409 si ya estaba o la coordenada es de otro registro; 400 si `<v>` es `-SNAPSHOT` (D-A10) o la ruta no cuadra |
| `PUT ….sha1` / `.md5` / `.sha256` / `.sha512` | 201 si coincide con lo recibido; 400 si no. No se guardan: se calculan |
| `PUT …/maven-metadata.xml` (y sus sumas) | 201 y **se tira**: la metadata se genera (P1) |
| `GET`/`HEAD` en la dirección del registro o en `…/artifacts/maven/…` (la de la celda) | los bytes; las sumas y `maven-metadata.xml`, generadas de lo que hay |
| cualquiera sin credencial | 401 con `WWW-Authenticate: Basic realm="ore"` (Maven no manda credenciales hasta que se le piden: P4) |

Rutas con `..`, vacías o fuera del alfabeto de una coordenada, 400. Cada publicación deja huella en
la actividad (`artefacto:publicar`: quién, registro, coordenada, versión, fichero).

**AR1·3 · los tokens** (D-A9, D-A12):

- `POST /artifacts/{base}/{schema}/{registro}/token` con el token del realm (la consola, o `curl`):
  puede el **dueño** del registro (0052) o quien tenga la potestad **`artefacto:publicar`**
  (`iam/migraciones/053`, a ORGADMIN y ACCOUNTADMIN), preguntado a `ore-iam`. Devuelve una vez
  `{usuario, token, caduca, settings, pom, comandos}`.
- `ore_art_<carga>.<hmac>`, la carga `{persona, registro, verbo, celda, caduca}`. Firma HMAC-SHA256
  con `ORE_ARTEFACTOS_SECRETO` o, sin él, una clave al azar por proceso (el patrón de
  `teselas.rs`): **reiniciar `ore-serve` invalida los tokens emitidos**. No se guarda.
- Al usarlo: firma, caducidad, celda y registro. Un token de publicar también lee su registro (el
  `GET` de la metadata que hace `mvn deploy`).
- **El de leer, para la capa:** `verbo: leer`, registro `*` (toda la celda), 20 min. Lo firma
  `ore-serve` al rendir el Job de la capa de la JVM y va en un hueco nuevo de la plantilla,
  `ORE_ARTEFACTOS_TOKEN`. **Se ve en el Job** (`kubectl get job`): por eso sólo lee y caduca pronto.
  `ore-serve` tolera una plantilla sin el hueco: **binario antes que malla**.

**AR1·4 · el registro se puede crear** (`documentos.rs`): `ArtifactRegistry` en el motor de
documentos (carpeta `registries/`), para crearlo desde la consola o la API antes de AR3. Quitar un
formato que tiene librerías, o borrar un registro con librerías, 409 (retirar es AR5).

**AR1·5 · la capa de la JVM lo usa** (`puesto/jvm/capa/`, imagen `capa-jvm`, `malla/55`):

- `resolver.sh`, con `ORE_ARTEFACTOS_TOKEN`: un `settings.xml` con el `<server>` de ORE y el espejo
  `maven-default-http-blocker` anulado (`mirrorOf` que no casa con nada), porque `ore-serve` es
  `http://` dentro de la celda (P4). `Capa.java pom` pone `…/artifacts/maven` **antes** de Central.
- **D-A6 en las directas:** antes de resolver, por cada dependencia declarada, `GET` de su
  `maven-metadata.xml` en la celda. Si la coordenada está reclamada y no tiene la versión pedida, el
  informe dice `estado: error` con «`com.banco:motor-riesgo:2.0.0` no está publicada en
  `banco.riesgo.libs`; una coordenada de un registro no se busca fuera».
- La plantilla `55` gana el hueco `ORE_ARTEFACTOS_TOKEN` y se empuja a la cola de victor con
  `malla/empujar-plantilla.py` (con `plantilla-capa-jvm.txt` entre lo que empuja), **después** de
  desplegar el binario.

**AR1·6 · la medida en victor** (`pruebas-de-fuego/artifacts/ar1.sh`):

1. Crear `banco.riesgo.libs` (`formats: [maven]`) por la API.
2. Pedir el token y publicar con `mvn deploy` (el `pom.xml` con `<distributionManagement>`) y con
   `deploy:deploy-file`, desde un Docker local, por la API pública (el túnel, `https`).
3. Republicar → 409; la misma coordenada en otro registro → 409; un token caducado o de otro
   registro → 401/403; `-SNAPSHOT` → 400.
4. Un repositorio `transforms-java` de victor declara `com.banco:motor-riesgo:1.0.0`; la capa lo
   resuelve desde ORE (el informe lo dice); un Build importa la clase y escribe su dataset.
5. Declarar `2.0.0` (no publicada) → el error de D-A6 en el informe de la capa.

**AR1 está listo cuando** los cinco puntos pasan en victor. Empujar, cuando lo digas, en este orden:
binario (`ore-serve`, `capa-jvm`) y migración `053` → plantilla.

### 5.4 · Los previos de AR0 (medidos el 2026-10-25)

Con un repositorio de juguete que apunta cada petición (`pruebas-de-fuego/artifacts/ar0/`:
`juguete.py`, `medir.sh`, el registro en `20261025-registro.jsonl`; Maven 3.9 con Temurin 21, uv y
twine en Python 3.14) y, P3, con un pod en t-victor.

| previo | pregunta | lo medido | lo que decide |
|---|---|---|---|
| **P1** | ¿qué peticiones hace `mvn deploy:deploy-file`? | **10**: `PUT` del jar, su `.sha1` y su `.md5`; del pom (lo genera), `.sha1` y `.md5`; `GET maven-metadata.xml` (404 la primera vez) y su `PUT` con sus dos sumas. Basic **preventivo** en los `PUT`. Un 409 y un 401 llegan al usuario tal cual (`status code: 409`) | jar y pom inmutables; las sumas se cotejan; `maven-metadata.xml` se acepta y se tira. Ningún paso dice «terminé»: la unidad es el fichero |
| **P2** | ¿cómo publican `uv publish` y `twine upload`? | **un `POST` multipart por fichero**, `:action=file_upload`, con `name`, `version`, `filetype`, `sha256_digest`, `blake2_256_digest` y el fichero; Basic con `__token__`. **El nombre llega sin normalizar, y distinto por fichero** (`Banco_Riesgo`, `Banco-Riesgo`). Republicar es 409 y uv lo enseña. `uv pip install` lee `GET /simple/<normalizado>/` (exige `Content-Type: text/html`), y `HEAD` + `GET` del fichero | el servidor normaliza (PEP 503); coteja `sha256_digest`; el índice simple con su `Content-Type` |
| **P3** | ¿llega el Job de la capa a `ore-serve`? | **sí**: un pod con `ore.dev/rol=driver` en t-victor da `/salud` → 200 y una ruta sin token → 401. **Y sale a internet el pod entero**: Central y PyPI → 200 | AR1 no toca la red; AR4 corta la salida del pod |
| **P4** | ¿qué hace Maven con el mismo GAV en ORE y en Central, ORE primero? | **⛔ con los ajustes de serie, Maven ni pregunta a ORE**: desde 3.8.1 bloquea todo repositorio `http://` (`maven-default-http-blocker`) **en silencio**, y todo sale de Central. Sin el bloqueo: lo que está en los dos sale de **ORE**; lo que sólo está en Central, de Central. Al leer, Maven no manda credenciales hasta un 401 | el `settings.xml` de la capa anula el bloqueo sólo para ORE; leer sin token es 401 con `WWW-Authenticate`; el orden no basta para D-A6 |

## 6 · Preguntas abiertas

- **¿Publicar desde un repositorio de código de ORE?** (una librería escrita en ORE que otros
  repositorios consumen). Encaja en el mismo registro; no entra en AR1–AR3.
- **¿Un token largo para el CI del cliente** (días o meses)? Pediría revocarlo uno a uno, es decir,
  guardarlo.
- **¿Cuota de almacén por celda?**
- **¿Escaneo de vulnerabilidades** de lo publicado y de lo espejado? Natural tras AR4.
- **Lo que se vio al construir AR0:** el volumen de `target` compartido entre worktrees (`/src` en
  todos) daba por fresco un binario de otra copia; se compila en un volumen propio por worktree.
