# 0056 · Versioning

**Estado:** **propuesto** (2026-10-05) · V0 medido, V1 hecho en el SDK; siguiente V2. **Decide:** qué es «la versión» de un activo
del catálogo —Dataset, View, MediaCollection, el ítem de media y la Function—, quién la pone, dónde
se guarda y cómo se lee lo de antes. Toca [`0033`](0033-el-dataset.md) (el dataset y sus
snapshots), [`0040`](0040-sql-views.md) (la vista y su copia), [`0046`](0046-ore-media.md)
(la colección y su manifiesto), [`0049`](0049-media-paradigms-in-code-repositories.md) (`apply()` y el
SQL anclado) y [`0050`](0050-functions-in-code-repositories.md) (D5: la versión de una función es la
del paquete).

## Lo medido (2026-10-04/05)

Leído en el código y, en victor, con `git log` sobre el espejo de `ore-serve` (sólo lectura).

| activo | la declaración | el contenido | leer lo de antes | *History* en la consola | el contrato |
|---|---|---|---|---|---|
| **Dataset** | commit git del YAML | snapshot Iceberg, por el puntero | **no**: el SDK lee siempre el `metadata_location` actual (`__init__.py:599-619`) | el último commit; los snapshots, que `GET /datasets/{b}/{n}` ya devuelve, no se pintan | ninguno: las columnas siguen a `write()` |
| **View** | commit git del YAML | el de su copia materializada (snapshot), si la tiene | no | el último commit | `ore diff` cubre columnas, pero sólo informa (`GET /ramas/{r}/cambios`); `WITH SCHEMA EVOLUTION` sí corta |
| **MediaCollection** | commit git del YAML | la transacción = snapshot del manifiesto | no hay lector de un estado viejo | las transacciones (lo mejor que hay) | — |
| **ítem de media** | — | `VersionId` de S3 o sha256 escrito; `digest` | **sí**: `stat(path, version=)`, con `current` | sin historia por ítem; Items no tiene «todas» | — |
| **Function** | commit git del YAML; `version` del paquete **congelada** (`test_project` en `0.1.0`, un commit en su historia) | **ninguna**: el cuerpo del `def` no deja rastro (cada `functions/*.yaml` de victor tiene 1 commit) | no | el último commit | `ore diff` sólo en la CLI (`main.rs:2276`) |

**Lo que se repite:**

1. **Dos versiones que no se conocen.** La de la declaración (git) y la del contenido (snapshot,
   transacción, digest). El índice publica sólo la de git como `version` (`ore-serve/assets.rs:127`,
   `git log -1`), y es la que pinta *History*.
2. **El contrato no se cumple en ningún sitio.** `ore diff` existe (`OOS5021`, «la versión deja de ser
   una afirmación y pasa a ser una comprobación») y ningún commit de la plataforma lo llama.
3. **El linaje no lleva versiones.** La procedencia de un transform guarda los nombres de lo que lee,
   no sus snapshots. La excepción es el testigo de la materialización —`p.n@<snapshot>` de todo lo
   que lee, `materializar.rs:1482`—: es el modelo.
4. **Lo que corre no tiene identidad completa** — el fallo de abajo.

### El fallo: el código nuevo de una función no recalcula

- **SQL anclado** (0049 B7·3): `apply(…, version="sql:"+huella)` (`__init__.py:1927`), y la huella es la
  consulta más el `spec` de cada función (`:1901`): **la firma**, que no cambia al cambiar el cuerpo.
  Se arregla `pdf_pages` → «4 saltados», y las filas viejas se quedan.
- **`apply(fn)` sin `version`**: el hash del texto de `fn` (`medios.py:474`), sólo de ella: un cambio en
  lo que llama no recalcula. Una función de `get_function` corre por `exec`, sin fichero: se hashea su
  bytecode.
- **`get_function` guarda en caché** por nombre (`_FUNCIONES`, `:815`): en una sesión larga, tras
  commitear, sigue corriendo el código viejo.

## Lo decidido

1. **Cada activo tiene dos versiones, con su nombre:**
   - **el contrato** — semver, **calculado** por las reglas de `ore diff` al entrar en `main`; nadie
     lo escribe;
   - **el contenido** — el que ya existe: snapshot (Dataset, copia de una View), transacción
     (MediaCollection), `version`/`digest` (ítem), y **`codeDigest`** (Function, nuevo).
2. **El commit git deja de llamarse «versión»**: es «el último cambio de la declaración», y así se dice
   en el índice y en la consola.
3. **Toda derivación y toda corrida guarda las dos** de lo que leyó y de lo que ejecutó, como el
   testigo de la materialización.
4. **El contrato informa; no corta.** La propuesta de fusión enseña el salto y a quién rompe. Corta lo
   que ya corta hoy (`WITH SCHEMA EVOLUTION`), nada más.
5. **Leer lo de antes es una forma para todos:** `read(name, version=…)` en el SDK y la tabla de
   versiones en *History*.

## Los pasos

| | qué | por qué aquí |
|---|---|---|
| **V0 · medir** | qué abarca `codeDigest` (el `def`, su módulo, lo que importa del repositorio, y cómo carga eso cada sitio donde corre); si la fusión de una propuesta en `main` pasa por el generador; cuánto cuesta | sin esto no se fija `codeDigest` ni el «se numera al fusionar» |
| **V1 · el código que corre** | `spec.codeDigest` en la Function; la huella del SQL y la versión de `apply()` lo usan; `get_function` invalida su caché si cambia | arregla el fallo de hoy; es el contenido de una función |
| V2 · el contrato de una Function | `metadata.version` por función, calculada (enmienda 0050 D5); con OOS, junto a la función fuera del paquete | necesita V1: el parche sale de `codeDigest` |
| V3 · el índice dice las dos | `contrato` y `contenido` por activo; el commit como «último cambio»; *History* con snapshots, transacciones y la historia de un ítem | el servidor ya tiene los snapshots |
| V4 · leer lo de antes | `read(…, version=)` y `sql` contra un snapshot o una transacción | los snapshots existen; falta la puerta |
| V5 · linaje con versiones | la procedencia de transforms y `write()` guarda `nombre@snapshot` y `codeDigest`, como el testigo | lo que deja decir «esto está viejo» fuera de las vistas |
| V6 · el contrato de los datos | semver calculado para Dataset, View y MediaCollection; `ore diff` en la puerta de las propuestas, informando | lo mayor, sobre todo lo de antes |

## Lo que esto no decide

- **Fijar versiones** al llamar (`functions.x@1`) o al leer: cuando alguien lo pida.
- **La retención** de lo viejo: `retention` y 0046.
- **Dos ramas que comparten una tabla del lago**: 0033.

## V0 · medido (2026-10-05)

| pregunta | medido | lo que decide |
|---|---|---|
| **¿qué corre de verdad?** | Los tres sitios donde corre una función cargan **un fichero**: el arnés de invocar (`exec(compile({fuente}, _FICHERO…))`, `funciones.rs:1123`), `get_function` (`__init__.py:844`) y la celda de SQL (por `get_function`). Ninguno pone el repositorio en `sys.path`; en TypeScript, el `import` relativo está «por hacer» (0050). | Lo que corre es **el fichero entero** más **sus dependencias**, no el `def` y no el repositorio |
| **¿qué importan hoy?** | Las 8 funciones de victor: sólo la biblioteca estándar y `ore` (`dataclasses`, `datetime`, `decimal`, `re`, `ore`, `ore.tipos`); ninguna importa otro módulo del repositorio. Tres comparten fichero (`medida_g3.py`: `eco_tipos`, `falla_siempre`, `valor_mal`). Cada repositorio declara su entorno en `pyproject.toml` o `package.json`. | Hashear el fichero cubre el 100 % de lo que hay. Compartir fichero hace que tocar una función mueva el digest de sus vecinas: es verdad (comparten módulo) y se acepta |
| **¿pasa la fusión por el generador?** | No. `fusionar` es el `merge` de la forja (`propuestas.rs`, `api.fusionar`): ni genera ni valida. El generador corre en **cada commit de una rama** (`arbol.rs:577`, y al sembrar y traer un repositorio, `repositorios.rs:304,531`). | El documento llega a `main` **ya generado en la rama**. Si dos ramas cambian la misma función, su YAML choca y la forja lo dice (`mergeable: false`): **no hace falta numerar al fusionar** — se calcula en el commit de la rama contra la versión de `main` |
| **¿cuánto cuesta?** | Un sha256 de un fichero de 0,2–3 KB, una vez por commit que lo toca. | Nada que medir más |

**`codeDigest`, fijado:** `sha256` de los bytes del fichero del `entrypoint` **y** del manifiesto de
entorno del repositorio (`pyproject.toml`/`package.json`, y su lock si lo hay), en ese orden. Cuando
una función pueda importar del repositorio (el `import` relativo de TypeScript, o un `sys.path` del
repositorio en Python), entra lo que importa — y no antes, porque hoy no corre.

**El salto de V2, fijado:** se calcula al commitear en la rama, contra el documento de esa función en
`main`; no en la fusión.

## V1 · el código que corre (hecho, en el SDK)

Sin tocar OOS: la identidad de lo que corre se toma **donde corre**, del fichero que se ejecuta.
`codeDigest` en el documento —con el manifiesto del entorno— llega con V2, que ya cambia la Function.

- `get_function` lee el código **cada vez** y reconstruye la función sólo si su digest cambió
  (`_FUNCIONES`: nombre → (digest, función)). Un commit nuevo se corre sin reiniciar la sesión.
- La función lleva `__ore_codigo__` (`codigo:<sha256 del fichero>`), y `apply(fn)` sin `version` lo
  toma antes que el bytecode del `def`.
- La huella del SQL anclado (`_huella_sql`) suma el código de cada función: otro cuerpo con la misma
  firma, otra huella → recalcula.
- `puesto/python/pruebas/test_version_del_codigo.py`: los cinco casos. (`test_repositorio` tiene 4
  fallos y 1 error que **ya estaban** con el SDK de `HEAD`: no son de esto.)

**Lo que cuesta una vez:** la huella cambia de forma, así que cada dataset anclado desde SQL que ya
existe (en victor, el de B7·3) **se recalcula entero en su siguiente pasada**. Es lo correcto: sus
filas se hicieron con un código que nadie había fijado.

**Para que llegue a un inquilino:** la imagen del puesto lleva el SDK; entra con la siguiente imagen
del CI.

## V2 · la función propia (en curso, rama `0056-versioning`)

### V2·1 · OOS v1alpha26 (hecho, `C:\oos` `81c66fd` + `56be6ae`, sin empujar)

La v1alpha25 ya era de otra entrega (`Transform`, con `OOS2046` y `OOS2047`), así que esta es la
**v1alpha26** y su código, **`OOS2048`** (un paquete llamado `functions`). Lo que decide: la
`Function` vive en `functions/<nombre>.yaml` de la raíz, se llama `functions.<nombre>` (único sin
mirar mayúsculas), su `entrypoint` es desde la raíz, `metadata.version` se calcula con las reglas de
v1alpha18 §7 contra la rama principal, y `spec.codeDigest` es la huella del fichero, su manifiesto
de entorno y su bloqueo, con los finales de línea en `\n`. 8 casos (2 aceptan, 6 rechazan) y 2 de
diff. Al implementarlo cambiaron dos cosas de la spec: `namespace` en una función propia es
`OOS1005` (la clave no existe), y los casos de diff no llevan paquete (uno sin cambios exigía su
propia versión).

### V2·2 · ore-core (hecho)

- `ApiVersion::V1Alpha26` —sin v1alpha25, como faltan v1alpha5 y 6—; las claves de la función
  propia; `qname()` = `functions.<nombre>`.
- `funcion_propia.rs`: `OOS1004` (versión, dueño y huella), `OOS2036` (fuera de `functions/`),
  `OOS2035` sin mayúsculas, `OOS2048`; y `huella()`.
- `promover`: el `entrypoint` de una propia es desde la raíz, y `codeDigest` se coteja (`OOS2013`).
- `generar`: **toda** función de código se genera ya como propia, con su `codeDigest` y su versión
  —`plan_con_anteriores` recibe los documentos de la rama principal; sin ellos, contra los del
  árbol—. Un documento de la forma de antes se mueve a `functions/` y **nace con la versión de su
  paquete**, sin salto por ganar la huella.
- `diff`: las funciones propias se comparan de una en una (`salto_de_funcion`), con `OOS5021` por
  función; la versión del paquete no las cuenta, y sólo se comprueba si hay paquete.
- `assets`: la función propia no tiene `paquete` (no está en ninguna base: se acaba el «+1» en
  `test_project.default`); su repositorio y su proyecto son los de su código.

Medido: ore-core y ore-code en verde; conformidad 88/88 y v1alpha26 8/8; el workspace entero en
verde salvo los 5 de siempre que necesitan `git` (la imagen de herramientas no lo trae).

**Lo que esto rompe hasta V2·3–V2·5, y por eso no entra en `main` todavía:** en cuanto un commit
regenera, la función pasa a `functions.<nombre>`, y los que la llaman por `<paquete>.<nombre>`
—`get_function`, el SQL de B7·2, las rutas de `ore-serve` y la consola— dejan de encontrarla.
Además `ore-serve` tiene que pasar a `generar` las versiones de `main` (`plan_con_anteriores`).

### V2·3 · ore-serve y SQL (hecho)

- **La versión contra `main`**: el commit de una rama le pasa a `generar` las funciones de `main`
  (`anteriores_de_main`, leídas del clon con `git show origin/main:…` —o `main` sin forja—: la propia
  con su texto, la de antes con la versión de su paquete). Así, varios commits en una rama no
  suben la versión varias veces.
- **`/funciones/{n}/resultados` y `/funciones/{n}/invocar`**: la función propia por su nombre; las
  rutas de dos y tres partes siguen (`/funciones/functions/{n}/…` es lo mismo). `ESCRITURAS` declara
  la nueva.
- **`/documentos/Function/functions/{n}`**: la ficha de la propia (su espacio es `functions`).
- **SQL**: `functions.<def>(…)` llama a la propia; el nombre de antes, `<paquete>.<def>(…)`, la
  sigue llamando si salió de ese paquete (`funcion_llamada`), y se registra por su nombre de verdad.
- El arnés de invocar ya encontraba el código: sube desde el documento hasta el primer
  `package.yaml` o la raíz, y desde la raíz el `entrypoint` es el de la propia.

Medido: ore-serve 160/161 —el que falla, `la_rama_del_puesto_nace_de_main_y_una_vez`, sólo falla
dentro del contenedor sobre un worktree (su `.git` es un fichero que apunta fuera); sobre una copia
del mismo código, pasa—; ore-core, ore-cli (conformidad y `functions generate`) en verde; `fmt` y
`clippy` limpios.

**Queda para V2·4:** `get_function("<def>")` en el SDK, y que lea el código desde la raíz —la ficha de
una propia no trae `paquete`, y hoy el SDK lo usa para componer la ruta—.
