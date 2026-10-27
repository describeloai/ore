# 0036 · Code Repositories

**Estado:** **aceptado · en vivo**, en construcción activa (imagen del 2026-10-04) · **Decide:**
qué es un code repository de ORE —**una instancia de una clase sobre una carpeta del árbol**—,
qué varía con su clase (el entorno, las capacidades, la interfaz) y cómo se acota todo lo demás
por su carpeta. Se apoya en [`0031`](0031-el-puesto.md) (el puesto) y
[`0035`](0035-el-proyecto.md) (el proyecto: el repositorio es su unidad de trabajo). Cada
lenguaje tiene su ADR: [`0037`](0037-language-servers-in-code-repositories.md) (el editor),
[`0039`](0039-sql-paradigms-in-code-repositories.md) (SQL),
[`0049`](0049-media-paradigms-in-code-repositories.md) (medios) y
[`0050`](0050-functions-in-code-repositories.md) (funciones).

## Qué es

Un repositorio son **tres cosas**, y todo lo demás se deriva de ellas:

| | es | de ahí sale |
|---|---|---|
| **el sitio** | una carpeta, `packages/<paquete>/<carpeta>` | sus ficheros, quién los tocó, sus ítems del catálogo, su sesión, su rama y sus propuestas |
| **la clase** | `plantilla` y `plantillaVersion` | el entorno, las capacidades y la interfaz |
| **el nombre** | «New Pipelines Java Transform» | cómo lo ven las personas |

Las tres viven en el **manifiesto**, el encabezado del `README.md` de la carpeta:

```markdown
---
nombre: New Pipelines Java Transform
plantilla: transforms-java
plantillaVersion: 6
---
La prosa es de quien la escriba.
```

- **La clave `plantilla` es lo que hace un repositorio.** Un README sin ella es una carpeta con
  README; el del propio paquete no es un repositorio.
- **El manifiesto no lleva configuración**: ni imagen, ni memoria, ni permisos. Lo que el
  repositorio usa va en el fichero de siempre de su ecosistema, y lo que la clase significa, en
  el producto.
- **Un árbol, no un git por repositorio.** La partición es por prefijo: el linaje, el catálogo y
  las referencias cruzan repositorios sin importar nada, y **todo lo acotado toma la carpeta
  como parámetro**.

## Las clases

La tabla es **del producto** (`ore_core::clases`), no del árbol: el árbol dice qué clase es
cada repositorio, y el producto sabe qué es esa clase hoy. Se agrupan en **familias**, y el
lenguaje es parte de la clase: dos lenguajes son dos plantillas, cada una con su versión.

| familia | clase | versión | entorno | escribe datos | nace con |
|---|---|---|---|---|---|
| **Transforms** | `transforms-python` | 6 | python | sí | `pyproject.toml`, `transforms/example.py` |
| | `transforms-java` | 6 | jvm | sí | `pom.xml`, `transforms/Example.java` |
| | `transforms-sql` | 6 | python | sí | `transforms/example.sql` |
| **Analytics** | `analytics-python` | 5 | python | no | `pyproject.toml`, `analysis/example.py` |
| **Models** | `models-python` | 5 | python | sí | `pyproject.toml`, `models/train.py` |
| **Functions** | `functions-python` | 10 | python | no | `pyproject.toml`, `functions/example.py` |
| | `functions-typescript` | 5 | node | no | `package.json`, `tsconfig.json`, `.gitignore`, la función y su prueba |
| **Semantics** | `semantics` | 1 | — | no, y **no ejecuta** | nada: lo suyo son documentos del árbol |

Las claves de antes (`transforms`, `analytics`, `models`, `functions`) resuelven a su clase de
Python, y no se ofrecen.

## ① El repositorio en el índice

`ore_core::repositorios` lee cada `README.md` con `plantilla` bajo `packages/`. El índice
(`/assets`) trae:

- **`repositorios[]`**: `ruta`, `nombre`, `plantilla`, `plantillaVersion`, `paquete`,
  `carpeta`, cuántos `items` tiene, el `manifiesto` y `version` (el último commit de la
  carpeta: commit, cuándo y asunto). De una clase conocida, además, `plantillaActual`,
  `actualizable` (`plantillaVersion` < la actual), `escribe` y `ejecuta`. Una clase que el
  producto no conoce se lista con `plantillaActual: null` y no rompe nada.
- **Un manifiesto roto se lista con su porqué** (`roto: "sin `nombre`"`) y no se queda ningún
  ítem.
- **`repositorio` en cada ítem**, en singular: un repositorio es el sitio donde se trabaja, y
  con dos anidados el ítem es del más hondo. (Un proyecto, en cambio, es una lente y se solapa.)
  De una `Function` de código, el repositorio es el de su código.
- **`clases[]` y `familias[]`**: la tabla del producto, con título y descripción de cada una,
  para que la consola no tenga una segunda copia.

## ② Los verbos

| | qué hace | un commit con |
|---|---|---|
| **`POST /repositorios`** `{paquete, carpeta, nombre, plantilla, proyecto?}` → 201 | crea el repositorio **entero** | el manifiesto, la semilla, las `Function` que la semilla define y, con `proyecto`, su `contiene` |
| **`PUT /repositorios/{ruta}`** → 200 | renombra o cambia de clase | el manifiesto; **la prosa se conserva** |
| **`POST /repositorios/{ruta}/actualizar`** → 201 | propone la plantilla de hoy | en una rama (`<persona>/plantilla-<ruta>-v<N>`): la semilla y el manifiesto con su versión; y **abre una propuesta** |
| **`DELETE /arbol/<carpeta>`** | borra | la carpeta entera, y el manifiesto con ella |

- **Crear**: 409 si la carpeta ya es un repositorio; 422 si la clase no existe (diciendo las que
  sí) o si el paquete es **de datos** —lo que trajo una fuente (`discover.*`): un repositorio va
  en el paquete de un proyecto—; 404 sin paquete o sin proyecto.
- **Actualizar es una propuesta, no un commit en `main`**: esos ficheros los ha editado alguien,
  y lo que se revisa es el diff. 409 si ya está al día o el producto no conoce su clase; 422 sin
  forja.
- **Desde un puesto, los tres son 403**: una sesión no crea repositorios. Con `main` protegida
  ([`0044`](0044-ramas-globales.md)), 423; con `X-Ore-Rama`, se escriben en esa rama.

## ③ La capa: las dependencias de cada repositorio

| entorno | fichero | qué se lee |
|---|---|---|
| python | `pyproject.toml` | `[project].dependencies` |
| jvm | `pom.xml` | `<dependencies>` |
| node | `package.json` | `dependencies` y `devDependencies` (no `file:`, `link:` ni `workspace:`) |

- **Lo declarado de un repositorio es suyo**: su capa suma la raíz del árbol, su paquete y cada
  carpeta hasta él. Lo de la raíz y lo del paquete es común a propósito; **lo de al lado, no**:
  el `torch` de un repositorio de modelos no lo baja el de análisis.
- **La capa se nombra por lo declarado** (`capa-<12 hex>`) y la resuelve un Job por entorno
  (`malla/52`, `55`, `57`); lo mismo declarado se resuelve una vez. Estados: `sin-dependencias`,
  `pendiente`, `lista`, `error`. Mientras se construye, abrir una sesión es 409.
- **La imagen manda**: lo que la imagen ya trae no hace capa, y una versión distinta de la de la
  sesión se dice en una frase. La capa va la última en el camino de Python y en el classpath (0037
  ③c).
- `GET`/`POST /entorno` con `X-Ore-Raiz: packages/<p>/<carpeta>` dice el de un repositorio.

## ④ Lo acotado por la carpeta

- **La sesión**: un puesto por persona, entorno y repositorio
  (`puesto-<persona>-<entorno>-<carpeta>-<hash>`), y su rama, `<persona>/<carpeta>`. Su ficha
  dice `repositorio`, `plantilla` y `escribe`.
- **El editor**: `GET /arbol` con `X-Ore-Raiz` lista sólo los ficheros del repositorio y dice
  `raiz`; la cabeza es la del árbol, porque el árbol es uno.
- **Las propuestas**: `GET /propuestas` con `X-Ore-Raiz` trae las que **tocan sus ficheros** (no
  las que se llaman de una manera), y una que la forja no sabe decir se enseña. `POST /propuestas
  {alcance}` abre una sobre un repositorio.
- **Un trabajo** (`POST /trabajos`) no es de ningún repositorio: corre un fichero y termina.

## ⑤ El techo y la versión

**Una clase quita, nunca concede.** Lo que gobierna sigue siendo la etiqueta, la declaración del
transform y quién escribe ([`0047`](0047-ore-access-control.md)); la clase sólo puede bajar ese
techo, y se aplica **en el servidor**, donde ya se decide quién escribe:

- **al abrir**: la sesión se abre en el entorno de su clase —pedir otro es 422— y una clase que no
  ejecuta (`semantics`) no abre sesión (422);
- **al escribir**: una clase que no escribe datos recibe 403 en el catálogo (`/v1`, todo lo que no
  es `GET`), en `datasets/…/confirmar` y al escribir una colección de medios, aunque su código lo
  declare.

**La versión** hace verdadera la columna «UPGRADE» de la consola: `plantillaVersion` contra la del
producto, y subirla es la propuesta de ②.

## ⑥ La consola

- **La lista** (`/repositories`): una fila por repositorio, del índice, con el icono de su clase,
  «last edited» de git y **UPGRADE**: «Up to date», «Propose upgrade to vN», o «—» si el producto
  no conoce la clase. Abrir una fila abre el workspace acotado a su carpeta.
- **Crear**: una tarjeta por familia, con la frase del servidor; una familia con varios lenguajes
  abre una tarjeta por lenguaje. El nombre es la carpeta, y la ubicación sale del árbol (el
  paquete del proyecto). Guardar es el `POST` de ②, y un 409 se enseña sin perder lo escrito.
- **El workspace**: el editor de la carpeta, con el servidor de lenguaje de su clase (0037), la
  sesión en su rama, y Run.

## ⑧ Las semillas

**Una instancia nace útil**: con la disposición, **un ejemplo que corre** y el fichero donde se
declara su entorno.

- **El ejemplo es código, en inglés**, que corre en cuanto sus referencias apuntan a algo del
  catálogo (`my_db.my_schema.my_dataset`, [`0038`](0038-assets-catalog-namespaces.md)), e importa
  lo que usa del SDK (`from ore import …`) para que el editor lo conozca.
- **El fichero de dependencias nace vacío a propósito**: lo que hace falta es el sitio donde
  declarar, no una dependencia que el ejemplo no usa.
- **Una semilla de funciones define funciones**: al crear, sus `Function` se escriben en el mismo
  commit (0050), y la de TypeScript trae su prueba.
- `transforms-sql` no trae fichero de dependencias: su consulta corre en DuckDB (0039).

## Aceptación

- `los-documentos.sh` **22** (crear en un commit, 409, 422, 404, `PUT` con la prosa, 403 desde un
  puesto, borrar, la capa propia al declarar) y **23** (el árbol y las propuestas acotados).
- `el-puesto.sh` **17** (`semantics` no abre; `analytics` lee y no escribe, ni a pelo ni con un
  transform), **6**, **6b** y **9c** (las capas de Python y de la JVM) y **18** (funciones de
  código en un repositorio).
- `la-propuesta.sh` **8d** (actualizar: rama y propuesta, `main` en su versión hasta fusionar,
  después al día y 409) y **8e**, **8f** (propuestas acotadas a un repositorio).
- `tests/assets.rs` (el repositorio y sus ítems en el índice, anidados y rotos) y
  `las_semillas_nombran_en_tres_partes`.
