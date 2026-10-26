# 0074 · Artifact Registries

**Las librerías propias de una organización, publicadas en la celda con sus herramientas de siempre y
usadas por sus repositorios como si vinieran de Maven Central o de PyPI.** Un Artifact Registry es un
activo del catálogo (`kind: ArtifactRegistry`, en `base.schema.nombre`) que guarda muchas librerías
Maven y PyPI. El cliente publica en él con `mvn deploy` o `twine upload`/`uv publish` y un token que
genera una persona desde la consola. Después declara la librería en el `pom.xml` o el
`pyproject.toml` de un repositorio de Code Repositories, la capa del repositorio la resuelve del
registro y sus Builds la importan.

**Estado:** **aceptado** (2026-10-26) · **en vivo en victor** (ORE `9d94849b`, OOS v1alpha34 `609cb6a`).
Cerrado de punta a punta para **Maven y PyPI**: crear el registro desde la consola, generar el token
como persona, publicar por el túnel, ver lo publicado con su autor, y que un Build de Java y uno de
Python importen la librería (§7). **Sin construir:** el espejo de Central y PyPI con la salida a
internet cortada (§9.1), retirar una versión y ver a quién afecta (§9.2), y npm. Sigue a
[`0037`](0037-language-servers-in-code-repositories.md) ③c (la capa de la JVM),
[`0050`](0050-functions-in-code-repositories.md) (las capas de Python y Node),
[`0038`](0038-assets-catalog-namespaces.md) (`base.schema.nombre`), [`0041`](0041-model-registry.md)
(un registro que es el catálogo), [`0052`](0052-ownership.md) (el dueño) y
[`0047`](0047-ore-access-control.md) (quién puede). Gramática: [OOS
v1alpha34](../../vendor/oos/spec/v1alpha34/01-artifact-registry.md).

## 1. Lo que se promete

1. **Nada con forma de ORE salvo la URL y el token.** Un equipo Java publica con su `mvn deploy`, sin
   plugin; uno Python, con `twine upload` o `uv publish`. Un repositorio declara la coordenada como
   declararía una de Central o PyPI.
2. **Lo publicado no cambia nunca.** Un fichero publicado no se reescribe ni se borra. Es lo que
   exigen Maven y PyPI, y lo que hace reproducible un Build.
3. **Una librería de un registro nunca se toma de fuera.** Si un registro de la celda tiene
   `com.banco:motor-riesgo`, ningún repositorio la resuelve de Central aunque alguien publique ahí
   una versión mayor (confusión de dependencias).
4. **Publica una persona, y queda dicho quién.** El token lo genera alguien con sesión en la consola,
   vive una hora, y cada versión publicada lleva su handle.
5. **Nuestros Builds no hablan con el servidor del cliente.** El cliente publica en la celda desde
   donde sí tiene acceso (su máquina, su CI); los Builds sólo leen de la celda. Es un puente, no una
   conexión a su Artifactory.

## 2. El registro y lo que contiene

```
sandbox.libs.banco                       ← el activo (ArtifactRegistry), en el árbol, con dueño
 ├─ maven  com.banco:calculo-riesgo      ← una librería: nace con su primer fichero (dato)
 │          └─ 1.0.0  .jar, .pom          ← versiones y ficheros, inmutables (dato)
 ├─ maven  com.banco:motor-riesgo
 └─ pypi   banco-utilidades
            └─ 1.0.0  .whl, .tar.gz
```

```yaml
apiVersion: oos.dev/v1alpha34
kind: ArtifactRegistry
metadata: { name: banco, namespace: sandbox, schema: libs }
spec:
  owner: team:riesgo        # opcional: sin él, quien lo crea (0052)
  # formats: [pypi]         # opcional: sin ella admite todos; con ella, restringe
```

- **El registro es un documento del árbol** y hereda todo del catálogo, como `Model` (0041): ruta
  única, dueño, permisos, búsqueda y su ficha en el Assets Catalog.
- **Admite todos los formatos.** Maven y PyPI conviven en el mismo registro, y npm entrará sin
  tocarlo. El formato de cada librería lo dice cómo se publica. `formats` sólo restringe.
- **Las librerías, sus versiones y sus ficheros son dato**, no árbol: nacen al publicar. Publicar
  una librería nueva no espera a una propuesta sobre `main`.
- **Una coordenada pertenece a un solo registro de la celda**, al primero que la publica. Otro
  registro no puede publicarla. Así se sabe de dónde servirla y quién responde de ella.
- **Se crea en cualquier rama y se publica en él desde `main`.** El texto de la consola:
  *An artifact registry is a global asset in the catalog. It can be created on any branch, but
  packages can only be published to it once it reaches `main`. From then on, code repositories on
  any branch can declare its packages as dependencies.* Lo publicado no vive en ninguna rama: es de
  la celda.

## 3. Cómo se usa

### 3.1 Crear

**«Create artifact registry»**, a la izquierda de «Create repository» en el home de Code
Repositories, y en el menú de crear de un schema del Assets Catalog (con la base y el schema ya
puestos). El modal pide la rama (`main` por defecto), la base, el schema, el nombre
(`^[a-z0-9][a-z0-9_-]*$`) y una descripción; no pide formatos. Escribe el documento con `PUT
/documentos/ArtifactRegistry/{base}/{schema}/{nombre}` en esa rama. Antes mira que no exista. En
`main` protegida devuelve el 423 de ORE tal cual. Fuera de `main` dice que se podrá publicar cuando se
fusione.

### 3.2 La ficha del registro

En el Assets Catalog, `ArtifactRegistry` es un tipo más (icono `RepoArtifact`, «Artifact registry»)
y aparece en su schema. Su ficha tiene tres pestañas:

- **Artifacts.** Una tarjeta por formato (*Maven artifacts*, *PyPI artifacts*, *npm packages*) con
  cuántos tiene. Al abrir una se ven sus librerías, y al abrir una librería sus versiones con
  fecha y **Created by** (el handle de quien publicó, `user:victor`). **Publish a new version** lleva
  a Publish con esa librería ya puesta. **Refresh** vuelve a pedir lo publicado; también se pide
  cada vez que se entra en la pestaña. En ORE lo publicado aparece en cuanto la subida acaba.
- **Publish.** El servicio (Maven o PyPI) y tres bloques:
  - **Token.** Se genera uno real al entrar, con la cuenta atrás de su hora. Se renueva solo 30 s
    antes de caducar, y «Generate new token» pide otro.
  - **Procedure**, listo para copiar y coloreado:
    - **Maven:** el `pom.xml` entero con su `<distributionManagement>`; «On your machine:» y el
      `settings.xml` con el token en `${env.ORE_TOKEN}`; `mvn deploy`.
    - **PyPI:** `pip3 install build twine`, `rm -rf dist && python3 -m build`, los `export` de
      `TWINE_USERNAME=__token__` y `TWINE_PASSWORD`, y `twine upload dist/* --repository-url …`.
    - **El último paso, en los dos:** «Once published, declare it in the `pom.xml` (o
      `pyproject.toml`) of your Java (o Python) code repository», con la dependencia.

    Desde **Publish a new version** el ejemplo lleva la coordenada de la librería y **la versión
    siguiente** (`1.0.0`→`1.0.1`, `1.0`→`1.1`, `2.0.0rc1`→`2.0.0rc2`), porque la última no se puede
    reescribir.
  - **Details.** El token y la URL del registro, para copiarlos sueltos.
- **Permissions.** Vacía todavía (§10).

Sin entrada propia en el sidebar: quien consume un registro es un repositorio de código, y su sitio
natural es al lado.

### 3.3 Publicar

```bash
export ORE_TOKEN=<token de Publish>
mvn -s settings.xml deploy                      # Maven

export TWINE_USERNAME=__token__ TWINE_PASSWORD=<token de Publish>
python3 -m twine upload dist/* --repository-url https://<celda>.ore.paladio.io/artifacts/<base>/<schema>/<registro>/pypi/
```

El usuario de Basic se ignora; el token decide. Republicar un fichero es un **409** que Maven y twine
enseñan tal cual. `-SNAPSHOT` es un 400.

### 3.4 Consumir

El repositorio declara la coordenada, y nada más:

```xml
<dependency><groupId>com.banco</groupId><artifactId>calculo-riesgo</artifactId><version>1.0.0</version></dependency>
```
```toml
dependencies = ["ore==1.0.0", "banco-utilidades==1.0.0"]
```

Al commitear el fichero de declaración cambia la **capa** del repositorio (su entorno con las
librerías bajadas). Un Job la resuelve, del registro antes que de Central o PyPI. Mientras tanto, un
Build dice *The repository environment is not ready yet* para no correr con el entorno viejo. La
consola enseña **un solo aviso fijo, con spinner**, desde el commit hasta «Session restarted with the
new libraries». El informe de la capa dice qué salió de un registro (`de_artefactos`).

## 4. Cómo funciona

### 4.1 `ore-serve` decide, `ore-medios` guarda

```
mvn deploy / twine ──https──► túnel ──► ore-serve ──flujo──► ore-medios ──► bucket del inquilino
                                         (decide)            (sha256/sha1/md5    ore/artefactos/…
                                                              al paso; crea,
                                                              nunca sobrescribe)
Job de la capa ──http──► ore-serve ──flujo──► ore-medios ──► bucket
                         (token de leer)
```

Es el reparto de 0049 aplicado a artefactos:

- **`ore-serve` decide** quién publica, en qué registro, si la coordenada es suya, si la versión vale
  y qué metadata responder.
- **`ore-medios` guarda y sirve** los bytes con una cuenta que sólo crea (`objectCreator`: no
  sobrescribe ni borra). La inmutabilidad la garantiza también IAM.
- **Los bytes atraviesan `ore-serve` en flujo**, con memoria constante, en las dos direcciones. Es la
  única excepción a «`ore-serve` no pasa bytes», y está acotada a `/artifacts/`. Fuera de la celda no
  hay otra puerta, y meter `ore-medios` en el túnel sería otra superficie pública.
- **`ore-entrada` contesta `100 Continue`** en las rutas en flujo. Maven lo pide en cada `PUT` y, sin
  él, espera 3 s por fichero. Por el túnel lo contesta Cloudflare.

### 4.2 Las rutas

| petición | qué hace |
|---|---|
| `POST /artifact-registries/{b}/{s}/{r}/token` | el token de publicar de una persona (§4.4) con las URL de Maven y PyPI |
| `GET /artifact-registries/{b}/{s}/{r}` (`x-ore-rama`) | la ficha: documento de esa rama, `owner`, `you_own`, `in_main`, formatos declarados y admitidos, y lo publicado por librería, versión y fichero (tamaño, sha256, `published_by` como handle, fecha). Lo publicado sólo se pregunta con el registro en `main` |
| `PUT /artifacts/{b}/{s}/{r}/maven/<g>/<a>/<v>/<fichero>` | el fichero en flujo a `ore-medios`: 201; 409 si ya estaba o la coordenada es de otro registro; 400 con `-SNAPSHOT`, un formato no admitido o una ruta que no cuadra |
| `PUT ….sha1` / `.md5` / `.sha256` / `.sha512` | 201 si casa con lo que calculó `ore-medios`, 400 si no; no se guardan |
| `PUT …/maven-metadata.xml` | 201, y se tira: la metadata se genera |
| `POST /artifacts/{b}/{s}/{r}/pypi/` (`:action=file_upload`) | multipart en flujo: todo se decide con los campos, antes del primer byte del fichero |
| `GET`/`HEAD /artifacts/maven/…` | de toda la celda: encuentra el registro por la coordenada; los bytes en flujo, y las sumas y `maven-metadata.xml` generados |
| `GET /artifacts/pypi/simple/<nombre>/` | de toda la celda: HTML de PEP 503 con `#sha256=` y `data-requires-python`; un nombre sin normalizar redirige al normalizado |
| cualquiera sin credencial | 401 con `WWW-Authenticate: Basic realm="ore"` |

Un nombre que no es de ningún registro es **404, nunca 403**: uv trata un 401 o un 403 como un
índice roto y tumba la capa entera.

### 4.3 Dónde viven los bytes

```
ore/artefactos/<base>/<schema>/<registro>/<formato>/<librería>/<versión>/<fichero>.json   la ficha: sumas, tamaño, quién, cuándo
ore/artefactos/coordenadas/<formato>/<coordenada normalizada>.json                        el registro que la reclamó
ore/artefactos/sha256/<hex>                                                                los bytes
```

- **En el bucket del inquilino**, cifrado con su clave, como el lago y las capas.
- **«Sube si no estaba»** da la inmutabilidad y la reclamación sin carreras. Primero se reclama la
  coordenada, después los bytes por suma y al final la ficha.
- **La unidad es el fichero, no la versión.** PyPI sube cada fichero en su propia petición y Maven
  sube jar, pom y sumas por separado; ninguno dice «terminé». Una versión es los ficheros que tiene.
- **Prefijo propio, no `ore/v2/blobs/`:** la recogida de blobs (0046) borra lo que ningún manifiesto
  nombra, y un jar publicado no lo nombra ninguno.
- **En las claves, `~` en lugar de `:`** (`com.banco~motor-riesgo`): con `:` la firma SigV4 no casa.
- **La coordenada se compara como la comparan las herramientas:** tal cual en Maven; en PyPI
  normalizada (PEP 503) y la versión sin ceros finales (PEP 440: `1.0` y `1.0.0` son la misma).

### 4.4 Los tokens

- **`ore_art_<carga>.<hmac>`**, firmado con HMAC-SHA256 con la clave de la celda
  (`t-<celda>-artefactos-secreto` en Secret Manager, traída al tmpfs de `ore-serve` al arrancar). Con
  clave fija, un reinicio de `ore-serve` no invalida los tokens vivos.
- **El de publicar lo pide una persona**, de un registro, y vive **1 h**. Al emitirlo, `ore-iam`
  decide, y su decisión viaja dentro del token. Cada publicación la cita en la actividad
  (`artefacto:publicar`) y en la ficha del fichero (`quien`, que la vista traduce a handle). Quitar
  el permiso no corta un token ya emitido; es una excepción acotada a la revocación en 30 s de 0047.
- **Quién puede pedirlo** (migración `053`): `artefacto:publicar` es de todo el que pertenece y basta
  para un registro propio; para el de otro hace falta `artefacto:gestionar` (ORGADMIN y
  ACCOUNTADMIN). Un agente no lo emite: de una sesión sólo pasan los verbos de datos.
- **El de leer es de la capa**: lo firma `ore-serve` al rendir el Job (`capa:<job>`, 1 h, toda la
  celda), *después* de nombrarlo, para que el nombre siga saliendo del hash de la plantilla.

### 4.5 Las capas

- **Java** (`malla/55-la-capa-jvm.yaml`, `puesto/jvm/capa/`): el `settings.xml` con el `<server>` y el
  bloqueo de `http://` anulado sólo para ORE (Maven ≥ 3.8.1 lo bloquea en silencio); ORE antes que
  Central. **La protección contra confusión de dependencias se construye** en `Capa artefactos`: por
  cada dependencia declarada se pide el `maven-metadata.xml` de la celda. Con un 404 la dependencia
  no es de ningún registro y sale de Central. Con 200 pero sin la versión pedida, la capa queda en
  `error` con el motivo y Maven no corre. Si la celda no contesta, tampoco se resuelve.
- **Python** (`malla/52-la-capa.yaml`, la misma plantilla para Spark y los notebooks):
  `UV_INDEX="ore=…"` antes que PyPI, con la credencial **por entorno**. **La protección viene de
  serie**: con `first-index`, uv no busca en PyPI un nombre que ORE tiene, tampoco en lo transitivo.
  No se toca `index-strategy`. Sólo se instalan ruedas (`no-build`). El sdist se acepta al publicar,
  pero no se usa.
- **El token no sale del contenedor que resuelve.** No se escribe en el directorio que comparte con
  el que sube, ni en la caché de uv, ni en el lock que vuelve al repositorio. El lock sí nombra la URL
  de ORE: es verdad y no es secreto.

## 5. Decisiones

| | decisión |
|---|---|
| **D-A1** | el activo del catálogo es el **registro**, que contiene muchas librerías |
| **D-A2** | ficheros inmutables: nunca se reescribe ni se borra uno publicado; una versión se retira (§9.2) |
| **D-A3** | el registro se declara en el árbol; librerías, versiones y ficheros son dato en el bucket del inquilino |
| **D-A4** | formatos `maven` y `pypi`; `npm` después; ni Conda ni Docker |
| **D-A5** | se publica en la dirección del registro; se lee de una por celda y formato |
| **D-A6** | una coordenada reclamada por un registro de la celda nunca se resuelve de fuera (Java: directas, construido; Python: también transitivas, de serie; Java transitivas con el espejo) |
| **D-A7** | en la consola, «Create artifact registry» en el home de Code Repositories y en el Assets Catalog; sin entrada en el sidebar |
| **D-A8** | la meta: el Job de la capa sólo habla con ORE, y sólo ORE sale, a orígenes cerrados (§9.1) |
| **D-A9** | el token de publicar lo pide una persona, se firma tras preguntar a `ore-iam` y vive ≤ 1 h |
| **D-A10** | sin `-SNAPSHOT`: una versión que cambia rompe la inmutabilidad |
| **D-A11** | se publica en cualquier coordenada nueva de un formato admitido; la librería nace con su primer fichero |
| **D-A12** | leer es de toda la celda: cualquier repositorio resuelve cualquier registro (por registro, con A8 de 0047) |
| **D-A13** | una coordenada es de un solo registro, el primero que la publica; moverla es retirarla y publicarla en el otro |
| **D-A14** | en PyPI la coordenada es el nombre normalizado (PEP 503) y las versiones se comparan normalizadas (PEP 440) |
| **D-A15** | el sdist se acepta al publicar (rechazarlo rompería `upload dist/*` a medias), pero la capa sólo instala ruedas |
| **D-A16** | un registro admite todos los formatos y `formats` sólo restringe (v1alpha34, cambiada sin versión nueva: afloja y no rompe un documento); se crea en cualquier rama y se publica en él cuando está en `main` |

## 6. Lo que decidió cada medida previa

Medido el 2026-10-25 con repositorios de juguete que apuntan cada petición
(`pruebas-de-fuego/artifacts/ar0/`, `ar2/`) y un pod en t-victor.

| pregunta | lo medido | lo que decidió |
|---|---|---|
| ¿Qué hace `mvn deploy`? | 10 peticiones: `PUT` de jar, pom y sus sumas, y `maven-metadata.xml`; Basic preventivo; `Expect: 100-continue` en cada `PUT` | la unidad es el fichero; las sumas se cotejan; la metadata se genera; `100 Continue` en `ore-entrada` |
| ¿Y `uv publish` / `twine`? | un `POST` multipart por fichero, **todos los campos antes del fichero**; el nombre sin normalizar y distinto por fichero | todo se decide antes del primer byte; normalizar y cotejar el nombre |
| ¿Maven con el mismo GAV en ORE y en Central? | con los ajustes de serie **ni pregunta a ORE** (bloqueo de `http://`); sin él, ORE primero, pero lo que ORE no tiene lo busca fuera | anular el bloqueo sólo para ORE; el orden no basta: D-A6 se construye en Java |
| ¿uv con el mismo nombre en «ORE» y en «PyPI»? | `first-index`: toma de ORE incluso lo pedido por una dependencia pública; `==9.9.9` falla sin preguntar a PyPI; ORE con 401/403/500 o caído falla siempre | D-A6 de serie en Python; un nombre ajeno es 404, nunca 403 |
| ¿Cuánto deja pasar el túnel? | 100 MiB (Cloudflare corta antes del origen) | un fichero de más no se publica por el túnel (§10) |
| ¿Llega el Job de la capa a `ore-serve`? | sí; y el pod entero sale a internet | la capa lee por `ore-serve`; cortar la salida es del espejo |

## 7. Lo medido en victor

**Publicar, por el túnel y desde un Docker local** (2026-10-25, con la clave de la celda):

| qué | resultado |
|---|---|
| `mvn deploy:deploy-file` de un jar pequeño | ✅ 21,9 s (10 peticiones) |
| un jar de 60 MB | ✅ ~14 s, con el temporal de `ore-medios` en un `emptyDir` |
| `uv publish` (rueda y sdist) · `twine upload` | ✅ 4,2 s · ✅ |
| republicar · `-SNAPSHOT` · sin token · token de otro registro · la coordenada en otro registro | 409 · 400 · 401 · 403 · 409 |

**De punta a punta, como un cliente** (2026-10-26, la consola contra victor y el usuario `victor`):

| paso | Java | Python |
|---|---|---|
| el registro | `sandbox.libs.banco`, creado desde el modal, sin `formats` | el mismo |
| el token | generado en Publish, por la persona | el mismo |
| publicar | `mvn deploy` de `com.banco:calculo-riesgo:1.0.0` (clase `com.banco.riesgo.Calculo`): **BUILD SUCCESS, 9 s** | `twine upload` de `banco-utilidades 1.0.0` (rueda y sdist) |
| en Artifacts | la versión, con **Created by `user:victor`** | lo mismo |
| declarar | `pom.xml` de `javatransformv5` con `calculo-riesgo`, `motor-riesgo` y `grande` (60 MB) | `pyproject.toml` de `pytransformsv5` con `banco-riesgo==1.0` y `banco-utilidades==1.0.0` |
| la capa | `la-capa-jvm-43fc908e7e33`, **50 s**: `lista`, `de_artefactos` = las tres | `la-capa-9e51a0f6bf97`, **21 s**: `lista`, `de_artefactos` = `banco-riesgo==1.0`, `banco-utilidades==1.0.0` |
| el Build | **Succeeded, 13 s**: `sandbox.exposicion_ajustada_jvm` = `{com.banco:calculo-riesgo, "calculo-riesgo 1.0.0", 1250}` | **Succeeded, 14 s**: `sandbox.exposicion_ajustada` = `{banco-utilidades, 1.0.0, 1250}` |

La versión y el 1250 los calculan `Calculo.version()`/`exposicionAjustada(1000, 0.25)` y
`banco_utilidades.__version__`/`exposicion_ajustada(1000, 0.25)`: código construido en la máquina del
cliente y que sólo existe en el registro. Los bancos locales, sobre un S3 local (VersityGW) con los
binarios de verdad, pasan **26 de 26** (`ar1-local.sh`, Maven) y **38 de 38** (`ar2-local.sh`, PyPI y
la capa de Python).

## 8. Dónde está cada pieza

| pieza | dónde |
|---|---|
| la gramática | OOS v1alpha34: `spec/v1alpha34/01-artifact-registry.md`, `schemas/v1alpha34/artifact-registry.schema.json` |
| el kind y su forma | `ore-core` (`Kind::ArtifactRegistry`, `document.rs`), el índice de assets (`artifactregistry:<b>.<s>.<n>`) |
| el documento, su dueño y sus guardas | `ore-serve/src/documentos.rs` (quitar un formato o borrar un registro con librerías, 409) |
| las puertas Maven, la ficha y los tokens | `ore-serve/src/artefactos.rs`; PyPI, `ore-serve/src/pypi.rs`; el handle, `acceso.rs` (`handle_de_persona`) |
| el multipart en flujo, `100 Continue`, el cliente en flujo | `ore-entrada` (`multipart`, `pedir_subiendo`, `pedir_bajando`) |
| los bytes | `ore-medios/src/artefactos.rs` (puerto 8097, sólo `ore-serve`) |
| las potestades | `iam/migraciones/053-las-potestades-de-los-artefactos.sql`, listada en `iam/kustomization.yaml`; `ci/migraciones-en-la-lista.py` falla el CI si una migración no está en la lista |
| las capas | `malla/55-la-capa-jvm.yaml` y `puesto/jvm/capa/` (`Capa artefactos`); `malla/52-la-capa.yaml` y `puesto/python/capa/` (`resolver.sh`, `capa.py informe`) |
| la clave | `aprovisionar-inquilino.sh` (crea `t-<celda>-artefactos-secreto`), `malla/40-ore-serve.yaml` (la trae a `/testigo/artefactos`) |
| el temporal de las subidas grandes | `malla/45-ore-medios.yaml` (`emptyDir` de 2 GiB en `/temporal`) |
| la consola | `components/repositories/CreateArtifactRegistryModal.tsx`; `components/catalog/AssetDetail.tsx`, `ArtefactosDelRegistro.tsx`, `PublicarRegistro.tsx`, `CodigoResaltado.tsx`; `lib/server/query.ts` |
| las pruebas | `pruebas-de-fuego/artifacts/`: `ar1-local.sh`, `ar2-local.sh`, `ar-victor.py` y los previos `ar0/`, `ar2/` |

**Operar:**
- **Rotar la clave** invalida todos los tokens vivos: versión nueva del secreto y `rollout restart`
  de `ore-serve`, a mano. El aprovisionador sólo la crea, nunca la rota.
- **Una plantilla de capa nueva** va a la cola de la celda (`malla/empujar-plantilla.py`) **después**
  de que la celda sirva el `ore-serve` que la rellena. Al revés, el hueco del token queda sin rellenar
  y la capa resuelve como sin Artifacts: no rompe, pero no lee del registro.

## 9. Lo que falta del producto

### 9.1 El espejo y cortar la salida

La celda hace de proxy de Central y PyPI: el resolver pide a ORE, y ORE baja de fuera la primera vez,
guarda el fichero con su suma y lo sirve desde ahí. Con eso el Job de la capa deja de salir a
internet; sólo sale ORE, a una lista cerrada de orígenes. Todo lo que entra en un Build queda
registrado y puede bloquearse. Lo transitivo de Java queda cubierto contra la confusión de
dependencias, y si Central cae o retira un paquete, los Builds siguen.

### 9.2 Retirar y consumidores

- **Retirar** una versión, con motivo (una vulnerabilidad). Los resolvers se niegan a usarla y el
  informe de la capa dice por qué. **No se borra**: lo construido con ella sigue siendo auditable.
- **Consumidores:** qué repositorios, transforms y datasets usan cada versión. Sale de los informes
  de las capas y se escribe como linaje del catálogo. Responde a «¿a quién afecta retirar
  `motor-riesgo 3.2.0`?» antes de retirarla.

### 9.3 npm

La tarjeta *npm packages* ya está en Artifacts, sin Publish. Entra con la capa de Node (0050) y la
misma puerta de bytes.

## 10. Deuda abierta

- **Permissions** de la ficha está vacía. Debe decir quién es dueño, quién publica
  (`artefacto:publicar` el dueño, `artefacto:gestionar` los demás) y quién lee (la celda, D-A12).
- **La huella `artefacto:publicar` en `iam.huella`** no se ha mirado en victor. La decisión viaja en
  el token y la ficha del fichero ya dice quién publicó.
- **Leer por registro** (A8 de 0047): hoy cualquier repositorio de la celda lee cualquier registro.
- **Ficheros de más de 100 MiB** no pasan el túnel: subida por partes, o un nombre fuera del proxy.
- **El `GET` de un fichero en `ore-medios`** lo lee entero en memoria antes de servirlo; debe ir en
  flujo desde el lago.
- **Un token largo para el CI del cliente** (días o meses) pediría guardarlo para revocarlo uno a uno.
- **`ore-entrada`:** un cuerpo de más de 1 MiB a una ruta que no va en flujo puede llegar como 503 o
  520 en vez de 413, porque se cierra sin leerlo. No afecta a `/artifacts/`.
- **Tres pruebas de `ore-serve`** (`copia`, `rutas`) fallan a veces en paralelo porque comparten
  directorios temporales.
- **Preguntas abiertas:** publicar desde un repositorio de ORE (una librería escrita en la celda que
  otros consumen); cuota de almacén por celda; escaneo de vulnerabilidades, natural tras el espejo.
