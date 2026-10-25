# 0074 · Artifact Registries

**Estado:** **borrador · hoja de ruta** (2026-10-25) · AR0 hecho en local; AR1 planeado, su previo (AR1·0) medido. Se trabaja
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
| **AR1** | Maven: publicar, servir, la capa de la JVM lo usa | un transform Java en victor importa una clase de un jar publicado | AR1·0–AR1·4 **hechos en local** (§5.3); AR1·5 (victor) por hacer |
| **AR2** | PyPI: publicar (`uv publish`/`twine`), servir `…/pypi/simple/`; la capa de Python lo usa | un transform Python importa una rueda publicada | plan en §5.5; AR2·0 **medido** (D-A6 viene de serie); AR2·1–4 esperando el go |
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

### 5.3 · AR1 · Maven, el enfoque

**La intención.** Un equipo Java publica su librería en un registro de ORE **con su Maven de siempre**
(`mvn deploy`, sin plugin ni paso propio de ORE), y un transform de victor la importa **como si
viniera de Central**. Nada de lo que el cliente hace tiene forma de ORE salvo la URL y el token.

**Lo que el código decide antes de diseñar** (leído el 2026-10-25):

| hecho | dónde | lo que impone |
|---|---|---|
| desde fuera de la celda **sólo se llega a `ore-serve`** (el túnel → `entrada-de-la-puerta-del-tunel`); `ore-medios` no es alcanzable desde fuera | `malla/45-ore-medios.yaml`, `40-ore-serve.yaml` | el `mvn deploy` entra por `ore-serve` |
| `ore-serve` **no pasa bytes ni enlaza TLS**, y su cuerpo es un `String` de ≤ 1 MiB (`CUERPO_MAXIMO`); sirve con `servir_con_flujos` | `ore-entrada/src/http.rs`, `ore-serve/src/main.rs` | un jar no cabe en el cuerpo de hoy; `ore-serve` no puede escribir en el bucket |
| **`ore-medios` es la puerta de bytes de la celda**: sube en flujo (`servir_con_subidas`, hasta 5 GiB) calculando la suma mientras llega, y su cuenta tiene `objectCreator` en el bucket **sin condición**: crea, **no sobrescribe ni borra** | `ore-medios`, `aprovisionar-inquilino.sh` | la inmutabilidad la garantiza también IAM; los bytes de un artefacto van donde ya van los de la celda |
| `ore-serve` habla con `ore-medios` por el 8097, y confía en él (`entrada-a-medios`) | 0049 | «`ore-serve` decide, `ore-medios` sirve» ya es el reparto |
| el Job de la capa (`rol=driver`) llega a `ore-serve:8080`, **no** a `ore-medios` | P3, `entrada-a-medios` | lo que la capa lee pasa por `ore-serve` |

**El enfoque: el reparto de 0049, aplicado a artefactos.** `ore-serve` **decide** —quién eres, qué
registro, qué coordenada, si es suya, si la versión vale, qué metadata responder— y **pasa el flujo**
a `ore-medios`, que **guarda y sirve** los bytes con su cuenta que sólo crea. `ore-serve` no guarda
ni un byte ni habla con el bucket: los bytes **atraviesan** `ore-serve` en flujo, con la memoria
constante, en las dos direcciones. Es la única excepción a «`ore-serve` no pasa bytes», y se acota:
sólo bajo `/artifacts/`, porque fuera de la celda no hay otra puerta y meter `ore-medios` en el
túnel sería otra superficie pública.

```
mvn deploy ──https──► túnel ──► ore-serve ──flujo──► ore-medios ──► bucket del inquilino
                                 (decide)            (sha256/sha1/md5    ore/artefactos/…
                                                      al paso; crea,     (objectCreator:
                                                      nunca sobrescribe)  no sobrescribe)
Job de la capa ──http──► ore-serve ──flujo──► ore-medios ──► bucket
                         (token de leer, D-A6)
```

**Los pasos.** Cada uno deja algo que funciona y se comprueba solo; en local con commit, salvo la
medida.

**AR1·0 · el previo: el camino de los bytes** — *para no construir sobre una suposición.* **Medido
el 2026-10-25** (`pruebas-de-fuego/artifacts/ar0/medir-expect.sh`; el túnel, con `curl --http1.1`
contra `victor.ore.paladio.io`).

| pregunta | lo medido | lo que decide |
|---|---|---|
| ¿Manda Maven `Expect: 100-continue` en el `PUT`? | **Sí, en cada `PUT`** (jar, pom, metadata). Si nadie contesta `100`, espera **3 s** antes de mandar el cuerpo: un `deploy-file` de 5 MB tarda **33,2 s** contra **3,7 s** sin `Expect` (`-Daether.connector.http.expectContinue=false`). `ore-entrada` no contesta `100 Continue` en ningún sitio | por el túnel no hace falta: **el borde de Cloudflare contesta `100 Continue` él mismo** (medido en las cuatro subidas). Dentro de la celda y en el laboratorio sí: `ore-entrada` contesta `100 Continue` en las rutas en flujo antes de leer el cuerpo (unas líneas en `leer_cabeza`/`atender`) |
| ¿Cuánto deja pasar el túnel en un `PUT`? | **100 MiB, el de Cloudflare.** 2 MB, 60 MB y 101 MB (96 MiB) **atraviesan** Cloudflare; 120 MB (114 MiB) lo corta Cloudflare con su `413` en HTML, antes del origen | **un fichero de más de 100 MiB no se publica por el túnel**: Publish lo dice y el ADR lo apunta como deuda (subida por partes, o un nombre fuera del proxy). Un jar de librería rara vez llega |
| ¿Puede `ore-serve` recibir y emitir en flujo a la vez? | **Ya puede.** `servir_con_flujos` **es** `servir_con_subidas` con «ninguna ruta en flujo», y `Salida::Bytes` (0049 B3·1) ya emite bytes en flujo. Lo que no lee el manejador se descarta, para que quien sube reciba la respuesta y no un reset | `ore-serve` pasa a `servir_con_subidas` con un `en_flujo` que sólo acepta `PUT /artifacts/…`; el resto, como hoy. Sin función nueva en `ore-entrada` |
| *(visto de paso)* | hoy, un cuerpo de más de 1 MiB a cualquier ruta de `ore-serve` recibe su `413`… **o un `503`/`520`**: `ore-serve` contesta sin leer el cuerpo, cierra, y Envoy, que seguía mandándolo, lo convierte en un reset (2 MB → 503, 60 MB → 520, 101 MB → el 413 de `ore-serve`) | no es de AR1 (las rutas de artefactos van en flujo), pero un `PUT` con la ruta mal escrita lo vería: **deuda** de `ore-entrada`, descartar hasta un tope antes de cerrar |

**AR1·1 · la puerta de bytes de los artefactos, en `ore-medios`** — *los bytes y su inmutabilidad
viven donde ya viven los de la celda.*

- `PUT /artefactos/<base>/<schema>/<registro>/<formato>/<librería>/<versión>/<fichero>` (8097, sólo
  `ore-serve`): lee en flujo, calcula `sha256`, `sha1` y `md5`, sube los bytes a
  `ore/artefactos/sha256/<hex>` y después el `.json` del fichero; si el `.json` ya estaba, 409. En el
  primer fichero de una librería, **reclama la coordenada** (`ore/artefactos/coordenadas/…`); si es de
  otro registro, 409. Devuelve las sumas.
- `GET` del fichero (los bytes) y del listado (librerías, versiones, ficheros con sus sumas).
- La prueba: en `ore-medios`, contra el almacén en memoria de sus pruebas.
- **✅ Hecho en local (2026-10-25)** — `crates/ore-medios/src/artefactos.rs`; el puerto de
  `ore-serve` (8097) pasa a `servir_con_subidas` con el `PUT /artefactos/…` en flujo. Seis pruebas
  (sumas de `hola` contra `sha256sum`/`sha1sum`/`md5sum`, 409 al republicar sin tocar lo primero, la
  coordenada del primer registro, el listado, rutas raras, lo que llega corto no se guarda).
  Lo que salió al medir y queda así:
  - **`:` no va en una clave del bucket**: con `com.banco:motor-riesgo` en la clave, la firma SigV4
    no casa con la del S3 (403). En las claves la librería va con `~` donde la coordenada dice `:`
    (`en_clave`; `~` no está en el alfabeto de una coordenada y ninguna URI lo escapa).
  - **La coordenada se reclama antes de recibir los bytes**: quien no la tiene no deja nada; una
    subida que se corta deja la coordenada reclamada para su registro, que es el mismo que la
    publicará al reintentar.
  - ⚠️ **Deuda**: el `GET` de un fichero lo lee entero en memoria antes de servirlo (un jar de
    librería son kB o pocos MB, y el túnel corta en 100 MiB); servirlo en flujo desde el lago, como
    `contenido::del_lago`.

**AR1·2 · la puerta Maven, en `ore-serve`** — *Maven habla con ORE como con cualquier repositorio,
y ORE decide cada petición.*

- `ore-serve` sirve con `servir_con_subidas`, en flujo sólo `PUT /artifacts/…` (AR1·0); `ore-entrada`
  contesta `100 Continue` en las rutas en flujo.
- Las rutas:

  | petición | respuesta |
  |---|---|
  | `PUT …/<base>/<schema>/<registro>/maven/<g>/<a>/<v>/<a>-<v>[-clasificador].<ext>` | el flujo a `ore-medios`; 201; 409 si ya estaba o la coordenada es de otro registro; 400 si `<v>` es `-SNAPSHOT` (D-A10), el registro no admite `maven` o la ruta no cuadra |
  | `PUT ….sha1` / `.md5` / `.sha256` / `.sha512` | 201 si coincide con lo que `ore-medios` calculó; 400 si no. No se guardan |
  | `PUT …/maven-metadata.xml` (y sus sumas) | 201 y **se tira**: la metadata se genera (P1) |
  | `GET`/`HEAD` en la dirección del registro o en la de la celda, `…/artifacts/maven/…` (que encuentra el registro por la coordenada) | los bytes, en flujo desde `ore-medios`; las sumas y `maven-metadata.xml`, generadas de lo que hay |
  | cualquiera sin credencial | 401 con `WWW-Authenticate: Basic realm="ore"` (P4) |

  Rutas con `..`, vacías o fuera del alfabeto de una coordenada, 400. Cada publicación deja huella
  en la actividad (`artefacto:publicar`: quién, registro, coordenada, versión, fichero).
- El registro y sus formatos, del árbol compilado.
- La prueba: `mvn deploy` de verdad (Docker) contra un `ore-serve` y un `ore-medios` locales con el
  almacén de disco, como el laboratorio de 0049.
- **✅ Hecho en local (2026-10-25)** — `crates/ore-serve/src/artefactos.rs` (la ruta de Maven, el
  token `ore_art_…` firmado con HMAC-SHA256 y verificado sin atajos de tiempo, Basic, la metadata
  generada con el orden de versiones de Maven, las sumas cotejadas), `ore-entrada` (`100 Continue`
  en las rutas en flujo; `pedir_subiendo` y `pedir_bajando`, el cliente en flujo, HTTP plano del
  clúster como el de siempre) y `servir_con_subidas` en `ore-serve`. Cinco pruebas unitarias.
- **La medida de punta a punta** (`pruebas-de-fuego/artifacts/ar1-local.sh`: Maven 3.9 en Docker →
  `ore-serve` → `ore-medios` → un S3 local, VersityGW, como R2): **12 de 12 en verde**.

  | qué | resultado |
  |---|---|
  | `deploy-file` de un jar con una clase | ✓; cada `PUT` 0,3–0,7 s contra el S3 local, **sin las esperas de 3 s** del `Expect` |
  | un jar de 20 MB (el camino del temporal de `ore-medios`) | ✓, 6,6 s el `PUT` |
  | republicar 1.0.0 · `-SNAPSHOT` · sin token · token de otro registro | 409 · 400 · 401 · 403 |
  | `mvn deploy` de un proyecto con `<distributionManagement>` | ✓ (jar y pom de 1.1.0) |
  | otro proyecto lo resuelve de `…/artifacts/maven` con **sumas estrictas** (`-C`) y el bloqueo de `http://` anulado sólo para ORE | ✓; el jar resuelto es el publicado byte a byte, y una clase que importa `com.banco.riesgo.Calculadora` compila y da 42 |
  | `maven-metadata.xml` | generado, con 1.0.0 y 1.1.0 |
  | la misma coordenada en otro registro de la celda | 409 |

  Lo que queda para AR1·3: la huella en la actividad (`artefacto:publicar`) va con la emisión del
  token; las sumas `sha512` no se guardan (Maven no las pide por defecto) y su `PUT` se acepta.

**AR1·3 · los tokens y el registro se puede crear** — *quien es dueño publica sin pedir permiso a
nadie más, y nadie sin permiso publica.*

- `POST /artifact-registries/{base}/{schema}/{registro}/token` (D-A9), con el token del realm:
  `ore_art_<carga>.<hmac>`, 1 h; devuelve `settings.xml`, el `<distributionManagement>`, la URL y los
  dos comandos. (Fuera de `/artifacts/`, que es de Maven y va por Basic: esta ruta va con sujeto.)
- `ArtifactRegistry` en el motor de documentos (`registries/`); quitar un formato con librerías o
  borrar un registro con librerías, 409.
- **✅ Hecho en local (2026-10-25).** Lo que se decidió al construirlo:
  - **El permiso, como Postgres (051).** `iam/migraciones/053`: `artefacto:publicar` es **de todo el
    que pertenece** (`iam.por_defecto`) y `artefacto:gestionar` de ORGADMIN y ACCOUNTADMIN. El dueño
    del registro (0052, su `owner`) pide su token con `artefacto:publicar`; para el registro de
    otro hace falta además `artefacto:gestionar`.
  - **La huella va por la decisión.** `artefacto:publicar` existe aunque lo tengan todos porque
    Maven no trae el token de la persona: la decisión de `ore-iam` al emitir viaja **dentro del
    token** (`d`), y cada publicación la cita en la actividad (`artefacto:publicar`, sobre
    `artifacts/<registro>/<coordenada>/<versión>/<fichero>`). `ore-iam` atribuye por ella —como los
    actos de un puesto (0047 A6.3)— mientras viva (24 h, más que el token). Emitir el token es otro
    acto (`artefacto:token`, en `ESCRITURAS`).
  - **Las guardas del registro** preguntan a `ore-medios` (`POST /artefactos/librerias`); si no
    contesta, 503: no se quita lo que no se sabe vacío.
  - La prueba local crece a **20 de 20** (`ar1-local.sh`): crear un registro por `/documentos`, el
    token emitido por la API con sus instrucciones, un registro que no existe → 404, y las cuatro
    guardas (quitar `maven` o retirar un registro con librerías → 409; uno vacío → 200).
  - **Lo que el banco no prueba:** sin `ore-iam` (`exigir` deja pasar) no se ve un 403 por potestad
    ni la huella llegar a `iam.huella`: es de la medida en victor (AR1·5).

**AR1·4 · la capa de la JVM lo usa** — *el transform no hace nada especial: declara la
dependencia y está.*

- El token de leer (`*`, 1 h) en el hueco `ORE_ARTEFACTOS_TOKEN` de la plantilla `55`
  (`ore-serve` tolera la plantilla sin él: binario antes que malla).
- `resolver.sh`: `settings.xml` con el `<server>` y el bloqueo de `http://` anulado sólo para ORE;
  `Capa.java pom`: ORE antes que Central.
- D-A6 en las directas: coordenada reclamada sin la versión pedida → el informe lo dice y no se
  busca fuera.
- **✅ Hecho en local (2026-10-25).** Lo que se decidió al construirlo:
  - **El token entra después de nombrar el Job.** El nombre del Job de la capa sale del hash de
    la plantilla rendida y es lo que impide encolar dos veces la misma capa: un token nuevo en cada
    rendición lo cambiaría. `rendir_capa` nombra y luego rellena (`artefactos::token_de_leer`, a
    nombre de `capa:<job>`, **1 h** y no 20 min: el plazo del Job más lo que espere en la cola).
  - **La plantilla trae la dirección** (`ORE_ARTEFACTOS_URL: http://ore-serve:8080/artifacts/maven`)
    y el hueco; `gen-inquilino.py --comprobar` exige ahora que el hueco esté en el que resuelve y
    **no** en el que sube, y `empujar-plantilla.py` empuja también `plantilla-capa-jvm.txt`.
  - **El token no sale del contenedor que resuelve:** el `settings.xml` va en su `$HOME` (`/tmp`),
    no en `/trabajo`, que comparte con el que sube. Un hueco sin rellenar (un `ore-serve` viejo
    con la plantilla nueva) no empieza por `ore_art_` y se lee como «sin Artifacts».
  - **D-A6, en `Capa artefactos`** (Java, sin `curl`: la imagen tiene JDK y no curl): por cada
    dependencia declarada, el `maven-metadata.xml` de la celda. 404 → no es de ningún registro,
    Central como siempre; 200 sin la versión → `estado: error` con el motivo, **y Maven no corre**;
    si la celda no contesta, tampoco se resuelve (no se sabe de quién es). El informe gana
    `de_artefactos`: lo que salió de un registro.
  - La prueba local crece a **26 de 26** (`ar1-local.sh` §7: `resolver.sh` y `Capa.java` de
    verdad, como en `capa-jvm:1`): la capa con `1.0.0` queda `lista` con el jar del registro y el
    de Central, el informe dice `de_artefactos`, el token no queda en `/trabajo`, `2.0.0` (no
    publicada) da el error de D-A6 sin buscarla fuera, y sin token la `1.0.0` no se encuentra.
  - *(visto de paso)* tres pruebas de `ore-serve` (`copia`, `rutas`) **fallan a veces en
    paralelo** y pasan en serie (`--test-threads=1`, tres de tres): comparten directorios
    temporales. No es de AR1; es deuda de las pruebas.

**AR1·5 · la medida en victor** — *el resultado, no las piezas.*

1. Crear `banco.riesgo.libs` (`formats: [maven]`).
2. Pedir el token y publicar desde un Docker local, por el túnel: `mvn deploy` (con
   `<distributionManagement>`) y `deploy:deploy-file`, un jar pequeño y uno de 60 MB (el túnel corta
   en 100 MiB, AR1·0).
3. Republicar → 409; la coordenada en otro registro → 409; token caducado o de otro registro →
   401/403; `-SNAPSHOT` → 400.
4. Un `transforms-java` de victor declara `com.banco:motor-riesgo:1.0.0`; la capa lo resuelve desde
   ORE; un Build importa la clase y escribe su dataset.
5. Declarar `2.0.0` (no publicada) → el error de D-A6 en el informe.

**El empuje, cuando lo digas, en este orden:** oos (`ee5a7fc`) → ORE (`ore-serve`, `ore-medios` y
`capa-jvm` salen de la misma CI) con la migración `053` → la plantilla `55` a la cola de victor.

**Lo que AR1 no hace, y dónde va:** PyPI (AR2, con la misma puerta de bytes); la consola (AR3); el
espejo y lo transitivo de D-A6 (AR4); retirar y consumidores (AR5); leer por registro (A8 de 0047).

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

### 5.5 · AR2 · PyPI, el enfoque (esperando el go)

**La intención.** Un equipo Python publica su librería en un registro de ORE **con su `uv publish` o
su `twine upload` de siempre**, y un transform de victor la importa **como si viniera de PyPI**. Lo
único con forma de ORE: la URL y el token (usuario `__token__`, como en PyPI).

**Lo que AR1 deja hecho y AR2 usa tal cual:** la puerta de bytes de `ore-medios` (`PUT
/artefactos/…/pypi/<librería>/<versión>/<fichero>`: la suma al paso, el `.json` del fichero, la
reclamación de la coordenada, 409 al reescribir; el formato ya es parte de la clave), el token de
publicar y su huella, las potestades (`053`), `ArtifactRegistry` con `formats: [pypi]` y sus guardas,
el token de leer de la capa (`token_de_leer`) y `pedir_subiendo`/`pedir_bajando`.

**Lo que el código y P2 deciden antes de diseñar:**

| hecho | dónde | lo que impone |
|---|---|---|
| publicar es **un `POST` multipart por fichero**, no un `PUT` con el cuerpo crudo | P2 | `ore-serve` tiene que leer multipart **en flujo**: el cuerpo fuera del flujo es ≤ 1 MiB (`CUERPO_MAXIMO`) |
| el nombre llega **sin normalizar, y distinto por fichero** | P2 | la coordenada es el nombre normalizado (PEP 503); el `name` del formulario y el del fichero deben normalizar igual |
| la capa resuelve con **uv 0.12.23 fijado**, `no-build = true` (sólo ruedas) y un `pyproject.toml` que escribe `capa.py proyecto` | `Dockerfile`, `puesto/python/capa/` | el índice de ORE se añade donde se escribe ese proyecto o por entorno (`UV_INDEX`); una librería publicada sólo como sdist **no se instala** |
| la capa de Python sirve también a Spark y a los notebooks (`entorno_de_la_capa`) | `ore-serve/src/cola.rs` | el hueco del token va en todas las plantillas que corren `puesto/python/capa/resolver.sh` |
| el lock exportado vuelve al repositorio (`lock-del-repositorio.toml`) | `resolver.sh` | **ni el token ni nada de la celda** debe quedar escrito en él |

**El enfoque: el de AR1, con otra puerta.** `ore-serve` decide —token, registro, nombre, versión, si
la coordenada es suya— y pasa el fichero en flujo a `ore-medios`, que guarda y sirve. Lo nuevo es
**leer el multipart en flujo** y **hablar PyPI** (PEP 503 al servir, PEP 440 al comparar versiones).

```
uv publish / twine ──https──► túnel ──► ore-serve ──flujo (sólo la parte `content`)──► ore-medios ──► bucket
   POST multipart                        (lee los campos, decide)                       ore/artefactos/…/pypi/…
uv (la capa) ──http──► ore-serve: GET /artifacts/pypi/simple/<nombre>/   (generado del listado)
                                  GET /artifacts/pypi/files/…            ──flujo──► ore-medios
```

**Los pasos.** Como AR1: cada uno deja algo que funciona y se comprueba solo; en local con commit
salvo la medida. Las pruebas, `cargo test -p` de los crates tocados y `ar2-local.sh`; la suite
entera, el CI.

**AR2·0 · el previo de uv** — *lo que decide si D-A6 se construye o viene de serie.* **Medido el
2026-10-25** con el uv de la imagen (0.12.23) y twine 7.0.0, contra dos índices de juguete —«ORE»
con Basic y un «PyPI» falso, nunca el real— que ignoran `Range` como `ore-medios`
(`pruebas-de-fuego/artifacts/ar2/`: `indices.py`, `medir.py`, `publicar.py`; los registros
`20261025-casos.jsonl` y `20261025-publicar.json`). Cada caso resuelve un proyecto como el de
`capa.py proyecto` (`uv lock` + `uv export --format pylock.toml`), con `UV_INDEX="ore=…"`,
`UV_INDEX_ORE_USERNAME`/`_PASSWORD` y el «PyPI» en `UV_DEFAULT_INDEX`. Sin Docker: un venv.

| pregunta | lo medido | lo que decide |
|---|---|---|
| **`first-index`**: `banco-riesgo` 1.x en «ORE» y 9.9.9 en «PyPI» | `banco-riesgo>=1` → **1.1.0 de ORE**; pedida por una pública (`app-dep`, de «PyPI») → **1.1.0 de ORE**; pedida por una de ORE → de ORE. `==9.9.9` → **error sin preguntar a «PyPI»** («the first index… use `--index-strategy unsafe-best-match`») | **⭐ D-A6 viene de serie y cubre lo transitivo.** En Python no se construye nada: basta con que ORE vaya antes que PyPI y no tocar `index-strategy` |
| un nombre que «ORE» no tiene | 404 → «PyPI», como siempre | lo que no es de ningún registro sale de PyPI |
| «ORE» contesta **401, 403, 500** o **no contesta** | **falla siempre, nunca pasa a «PyPI»**: 401 y 403 → «unsatisfiable» con la pista del índice; 500 → tres reintentos y error; sin conexión → error. **Y falla también para un paquete sólo público** (403 en `utilidad`) | sin confusión de dependencias con ORE caído. **Pero un 401/403 tumba la capa entera**: `ore-serve` contesta **404**, nunca 403, a un nombre que no es de ningún registro o que el token no puede leer (cuando llegue A8); 401 sólo si el token no vale |
| la credencial | **de entrada**, en la primera petición a ORE (`capa`), y **nunca a «PyPI»**; `http://` sin queja | `UV_INDEX_ORE_USERNAME`/`_PASSWORD` por entorno; el `WWW-Authenticate` se mantiene pero uv no lo necesita |
| lo que queda en el lock | **ni el token ni el usuario**; **la URL de ORE sí** (en `uv.lock` y en el `pylock.toml` exportado) | el lock que vuelve al repositorio dice de dónde salió cada paquete (`http://ore-serve:8080/artifacts/pypi/…`): se deja, es verdad y no es secreto |
| ¿JSON (PEP 691)? ¿`Range` (sin PEP 658)? | pide `application/vnd.pypi.simple.v1+json` primero y **acepta el HTML**; **ningún `Range`**: `HEAD` y `GET` de la rueda entera | basta el HTML de PEP 503; ni PEP 691 ni PEP 658 ni `Range` en AR2 |
| el orden del multipart | **uv y twine mandan todos los campos antes del fichero** (`content`, el último); `Content-Length`, ni `chunked` ni `Expect` | **`ore-serve` decide antes de pasar un byte**: token, registro, nombre, versión, `sha256_digest` y `requires_python` llegan primero; el fichero, después, en flujo |
| el nombre al publicar | el `name` llega tal cual (`Banco_Riesgo`, `banco.riesgo`); el fichero de `uv build` sale normalizado (`banco_riesgo-1.0-…`). **`uv publish` se salta en silencio** (aviso y código 0) un fichero de nombre sin normalizar | se normaliza el `name` y se coteja con el del fichero; un fichero hecho a mano puede «publicarse» sin subir nada: Publish dice que se construya con `uv build` |
| `version = "1.0"` | `uv build` deja **`1.0`** (no `1.0.0`) en el fichero y en el campo | PEP 440: `1.0` y `1.0.0` son la misma versión con ficheros distintos: se comparan sin ceros finales (D-A14) |
| republicar | 409 → uv sale con **código 2 en el primer fichero**, con «File already exists» | republicar `dist/*` con lo viejo dentro falla en lo viejo: Publish limpia `dist/` antes de construir |

**AR2·1 · el multipart en flujo, en `ore-entrada`** — *una pieza genérica, como `pedir_subiendo`.*
Un lector que recorre un `multipart/form-data` sobre el `Read` de la subida: devuelve los campos
pequeños como texto (con tope) y, al llegar a la parte del fichero, un `Read` que termina en el
límite, sin cargarla en memoria. Pruebas: límites partidos entre lecturas, `CRLF` dentro del
fichero, campos después del fichero, un cuerpo cortado.

**AR2·2 · la puerta PyPI en `ore-serve`** — *publicar y servir.*
- `POST /artifacts/{base}/{schema}/{registro}/pypi/` (`:action=file_upload`): token de publicar de
  **ese** registro y `pypi` en sus `formats`; **los campos llegan antes que el fichero** (AR2·0), así
  que todo se decide antes del primer byte: el nombre normalizado (PEP 503), igual el del
  formulario y el del fichero; la versión normalizada (PEP 440); `filetype` `bdist_wheel` o
  `sdist`; el fichero en flujo a `ore-medios` con `pedir_subiendo`; `sha256_digest` cotejado con la
  suma de `ore-medios` (si no casa, el fichero no queda); `requires_python` al `.json` del fichero.
  Republicar → 409; la coordenada de otro registro → 409 (D-A13); la huella `artefacto:publicar`.
- `GET /artifacts/pypi/simple/<nombre>/` (de toda la celda, D-A12) y `GET
  /artifacts/{base}/{schema}/{registro}/pypi/simple/<nombre>/` (de uno): HTML de PEP 503 con
  `#sha256=` y `data-requires-python`, `Content-Type: text/html`, generado del listado (uv pide
  JSON primero y acepta el HTML). Un nombre sin normalizar redirige al normalizado, como PyPI.
  **Un nombre que no es de ningún registro: 404, nunca 403** (un 403 tumba la capa entera, AR2·0).
- `GET` del fichero, en flujo desde `ore-medios` (`pedir_bajando`).
- El token de `POST /artifact-registries/…/token` devuelve también las instrucciones de PyPI
  (`uv build --wheel`, `uv publish` y `twine upload` con `__token__`): es lo que la página Publish
  (AR3) pinta.

**AR2·3 · la capa de Python** — *un transform importa una rueda publicada.*
- El hueco `ORE_ARTEFACTOS_TOKEN` y `ORE_ARTEFACTOS_URL` (`…/artifacts/pypi/simple/`) en las
  plantillas que corren el resolver de Python; `rendir_capa` lo rellena para los entornos de la capa
  de Python, igual que para `jvm` (después de nombrar el Job). `gen-inquilino.py --comprobar`, como
  en AR1·4.
- `resolver.sh`: con un token `ore_art_…`, el índice de ORE antes de PyPI y su credencial **por
  entorno, nunca en un fichero del trabajo** (que comparte el contenedor que sube).
- **D-A6: nada que construir** (AR2·0): uv con `first-index`, ORE antes que PyPI, ya no toma de
  PyPI lo que un registro tiene, tampoco en lo transitivo. `capa.py` no toca `index-strategy`. El
  informe gana `de_artefactos`: los paquetes del `pylock.toml` cuyo origen es ORE.
- El lock que vuelve al repositorio: sin token (medido); con la URL de ORE, que se deja.
- `ar2-local.sh` (sobre el banco de AR1: VersityGW y los tres binarios): `uv publish` y `twine
  upload` de una rueda y su sdist, 409 al republicar, la coordenada en otro registro, el índice que
  lee `uv pip install`, y la capa de verdad (`resolver.sh` y `capa.py`) con 1.0.0, con 2.0.0 sin
  publicar y sin token.

**AR2·4 · la medida en victor, junto con AR1·5** — *el resultado.*
1. `banco.riesgo.libs` con `formats: [maven, pypi]`.
   (Lo que AR2·0 no puede medir en local: el túnel con un `POST` multipart grande.)
2. Publicar desde un Docker local, por el túnel, con `uv publish` y con `twine upload`.
3. Republicar → 409; token de otro registro → 403; `pypi` quitado de `formats` con librerías → 409.
4. Un repositorio Python de victor declara `banco-riesgo==1.0.0`; la capa lo resuelve desde ORE y un
   Build importa el módulo y escribe su dataset.
5. `banco-riesgo==2.0.0` (no publicada) → error en el informe, sin buscarla en PyPI.

**Decisiones que AR2 añade** (a la tabla del §4 cuando se construya):
- **D-A14** · la coordenada PyPI es el nombre normalizado (PEP 503) y las versiones se comparan
  normalizadas (PEP 440, sin ceros finales: `1.0` = `1.0.0`); dos ficheros cuyo nombre normaliza
  igual son de la misma librería.
- **D-A15** · el sdist **se acepta** al publicar (`uv publish dist/*` sube los dos y rechazar uno
  rompería el comando a medias), pero la capa sólo instala ruedas (`no-build`): Publish lo dice.

**Lo que AR2 no hace:** el espejo de PyPI y cortar la salida (AR4); `yank` (PEP 592), que es retirar
(AR5); PEP 658/691 y `Range` (uv no los necesita, AR2·0); `uv pip install` desde el terminal de una
sesión (la sesión recibe la capa ya resuelta).

## 6 · Preguntas abiertas

- **¿Publicar desde un repositorio de código de ORE?** (una librería escrita en ORE que otros
  repositorios consumen). Encaja en el mismo registro; no entra en AR1–AR3.
- **¿Un token largo para el CI del cliente** (días o meses)? Pediría revocarlo uno a uno, es decir,
  guardarlo.
- **¿Ficheros de más de 100 MiB?** El túnel de Cloudflare los corta (AR1·0): subida por partes, o un
  nombre fuera del proxy.
- **¿Cuota de almacén por celda?**
- **¿Escaneo de vulnerabilidades** de lo publicado y de lo espejado? Natural tras AR4.
- **Lo que se vio al construir AR0:** el volumen de `target` compartido entre worktrees (`/src` en
  todos) daba por fresco un binario de otra copia; se compila en un volumen propio por worktree.
