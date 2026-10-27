# 0050 · Functions in code repositories

**Estado:** aceptado. **Python y TypeScript, en vivo** en `victor`, de punta a punta —escribir la
función, probarla en Dry Run sin commit, publicarla al commitear, verla en Assets e invocarla—. Y
**el repositorio entero como entorno** (L para TypeScript, 2026-10-03/05; P para Python,
2026-10-05): lo que nace con él, sus dependencias con su lock, sus pruebas, sus tipos, su sesión y
el panel Libraries; ver [El entorno de un repositorio](#el-entorno-de-un-repositorio). Abiertos:
el operador `function` de Pipelines, las funciones `over` en Dry Run y lo que se lista al final.

Construye sobre lo que ORE ya define —la `Function` de OOS (v1alpha10–14), el puesto
([`0031`](0031-el-puesto.md)), la versión del paquete (`91-versioning`), la red por política con
nombre (0031 §7) y el acceso ([`0047`](0047-ore-access-control.md))— y **ejecuta** la enmienda a
[`0029`](0029-donde-corre-una-funcion.md) que 0031 W3.8 dejó escrita.

## Qué es

**Una función es código tipado y marcado.** En Python, un `def` con anotaciones y `@function`; en
TypeScript, la exportación por defecto de un `.ts` de `functions/`. La persona escribe sólo eso. La
plataforma **deriva** de la firma el documento `Function` —el contrato publicado—, lo escribe en el
mismo commit, lo enseña en Assets y lo hace cumplir igual en todos los sitios donde la función corre.

```python
# packages/ventas/facturacion/functions/quote.py
from ore import function
from ore.tipos import DateTimeTz, Money

@function(timeout="30s")
def quote_order(order: Order, cutoff: DateTimeTz, discount: Decimal = Decimal("0")) -> Quote:
    """Quote an order: its total in EUR and until when the quote holds."""
```

```ts
// packages/ventas/facturacion/functions/quoteOrder.ts
import type { Decimal, Money } from "ore";

export const config = { reads: ["ventas.pedidos"], timeout: "30s" };

/** Quote an order: its total in EUR. */
export default async function quoteOrder(orderId: string, discount: Decimal<5, 2> = "0"): Promise<Money<"EUR", 2>> { … }
```

```
 editor ──texto──► POST /funciones/firma ──► la firma, en vivo ──► Dry Run (formulario por tipo)
   │                    (ore-code)                                   │ Dry run: una celda en
   │                                                                 ▼ la sesión, con el contrato
   └─commit──► ore-serve deriva y escribe packages/<p>/functions/<nombre>.yaml en el MISMO commit
                    │
                    ├─► Assets → Functions: observa y configura (no ejecuta)
                    └─► se invoca: POST …/invocar (un trabajo del puesto) · ore.get_function() desde código
```

### Los principios

1. **El código manda; el documento se deriva.** Nadie escribe la función en YAML. El `Function`
   sale de la firma **leyendo el fichero, sin ejecutarlo** (`ore-code`), y el compilador lo coteja:
   `OOS2013` si el documento no es el que el código da. Uno escrito a mano, sin marca, sigue
   valiendo y no se toca.
2. **La firma habla OOS.** Los tipos de los parámetros y de la vuelta son los del catálogo
   —`Money<EUR, 2>`, `DateTimeTz`, `Struct`…—, no los del lenguaje. Lo que no se traduce no se
   deriva (`OOS2043`), con su ayuda.
3. **El contrato se cumple igual en todas partes.** La misma pieza por lenguaje —`ore.contrato` en
   Python, `contract` de `ore` en Node— convierte la entrada y comprueba la salida en Dry Run, en la
   sesión, al invocarla y al llamarla desde código: lo que funciona en un sitio funciona en los demás.
4. **Una función publicada es un nombre del paquete,** `<paquete>.<nombre>`, en el lenguaje que sea:
   vive en `packages/<paquete>/functions/<nombre>.yaml`, nunca junto al código.
5. **Se ejecuta en el puesto, en la celda;** nunca en `ore-serve` (0029 ①).
6. **Lo publicado tiene versión, y la decide `ore diff`,** la del paquete; no hay versiones por
   función.
7. **Sólo se alcanza lo declarado:** `over`/`reads` para los datos, `models` para los modelos; la
   red se abre por política con nombre. Las dependencias, por el entorno del repositorio.
8. **Assets observa y configura; no ejecuta.** Una función se ejecuta desde donde se usa —Pipelines,
   el código—, y se prueba donde se escribe (Dry Run).
9. **Cada lenguaje, con su marca y la misma regla.** Python decora; TypeScript no decora funciones
   sueltas y marca con su estándar, la exportación por defecto. Lo demás —derivar, cotejar, el
   documento, Dry Run, invocar— es una sola regla.

## El contrato

### El documento

La `Function` de OOS. `input` y `output` **son** la firma; lo que v1alpha18 y v1alpha23 añadieron:

| | qué | por qué |
|---|---|---|
| `runtime: python` | el código es un `def` de Python | la promoción de 0031 W3.8 |
| `runtime: node` (v1alpha23) | el código es la exportación por defecto de un `.ts` | la misma regla, para TypeScript (R3) |
| `entrypoint` | `<ruta>.py:<def>`; con `node`, `<ruta>.ts` (el fichero **es** la función) | desde la carpeta del paquete |
| `models: [modelo/<ref>]` | los modelos que el código **puede** llamar | `model` sigue siendo `runtime: model`; una función de código **usa** modelos, y lo usado se declara |
| `output` como valor | `{type: T}` si la vuelta no es un objeto | una función puede devolver un escalar o una lista |
| `limits.timeout`, `over`, `reads`, `models` | argumentos **literales** de `@function(...)`, o claves de `export const config` | se leen sin ejecutar |
| `owner` (v1alpha21) | quién responde de la función: **quien la crea**, `user:<handle>` | no sale del código, así que `OOS2013` no lo compara; al regenerar se conserva (0052) |

El documento derivado declara **la versión más baja que lo describe**: v1alpha18; v1alpha20 si la
firma usa sus tipos; v1alpha21 si lleva `owner`; **una de TypeScript, siempre v1alpha23**, la
primera con `node`. Generado en local, sin plataforma, no lleva `owner` y sus bytes son los de
siempre.

### Los tipos de la firma

| OOS | Python | TypeScript (v1alpha23) |
|---|---|---|
| `String` · `Boolean` | `str` · `bool` | `string` · `boolean` |
| `Float` | `float` | `number` |
| `Integer` | `int` | `Integer` (un `number` que el contrato exige exacto, ±2⁵³−1) o `bigint` |
| `Decimal` · `Decimal<p, s>` | `Decimal` · `Annotated[Decimal, Precision(p, s)]` | `Decimal` · `Decimal<p, s>` (una **cadena** con sus cifras) |
| `Money<EUR, 2>` · `Quantity<km, 1>` | `Money["EUR", 2]` · `Quantity["km", 1]` | `Money<"EUR", 2>` · `Quantity<"km", 1>` (cadenas) |
| `Date` · `Time` · `DateTime` | `date` · `time` · `datetime` | `LocalDate` · `LocalTime` · `LocalDateTime` (cadenas ISO, sin zona) |
| `DateTimeTz` | `ore.tipos.DateTimeTz` | `Date` (un instante) |
| `Opaque` | `bytes` | `Uint8Array` (base64 en JSON) |
| `list<T>` · no requerido | `list[T]` · `T \| None` | `T[]` · `T \| null`, `x?: T` |
| `Struct<…>` | una `@dataclass` del fichero | un `interface`, un `type` o un literal de objeto del fichero |
| `Media<c>` | `Media["c"]` | `Media<"c">` |

Los tipos de `ore` son **anotaciones**: en Python un `Annotated`, en TypeScript **alias** de
`number` y `string` (`const d: LocalDate = "2026-10-03"` no pide conversión). La derivación los lee
por su nombre y su origen; el contrato comprueba el valor. Fuera por ahora: una `Entity` como
parámetro, `set`/`dict`/`Map`/`Record`, tuplas, uniones de literales, `Vector`/`Anchor`, identidad y
marcas, y —en TypeScript— un tipo de otro fichero.

**Por qué estos nombres en TypeScript:** un `number` es un `double`, así que un entero lo dice
(`Integer`, `bigint`) y un decimal nunca es un `number`: los almacenes que pasan un entero de 64 bits
o un decimal como `number` pierden cifras por encima de 2⁵³ sin avisar. Y un `Date` es un instante:
una fecha de calendario es otra cosa, y convertirla supone una zona.

### La derivación: `ore-code`

Un crate propio, el único sitio que entiende el código del cliente, **sin ejecutarlo**:

- **Python** con `ruff_python_parser` (fijado; 655/655 ficheros de la stdlib de 3.14), resolviendo
  los nombres como Python —alias, sombras, comillas, `from __future__ import annotations`, la
  evaluación de 3.12— y con un oráculo diferencial sobre el `ast` de CPython.
- **TypeScript** con `oxc_parser` (fijado, 0.152; elegido frente a swc: 1238/1238 ficheros reales
  frente a 1228, 526 ms frente a 683, sin nada vetado). Sin el compilador de TypeScript: no se infiere
  nada, se lee lo escrito, y por eso la vuelta se anota. El TypeScript del runtime es el **borrable**
  (`--erasableSyntaxOnly`, lo que Node 24 ejecuta): un `enum`, un `namespace` con valores, las
  propiedades de parámetro, `import =` y `export =` son `OOS2043`, estén donde estén.
- **Nunca se para en el primer problema:** sintaxis, tipos sin traducción y valores no literales son
  datos, todos a la vez, con su sitio en el fuente.
- **Es hostil-seguro:** pila dimensionada por byte, **medida** (Ruff 168 B/byte; oxc hasta 1342, y se
  reservan 1536) y un límite de tamaño (1 MiB en Python, 256 KiB en TypeScript).
- Los fallos: `OOS2042` (el `entrypoint` no nombra un `def` síncrono, o el `.ts` no exporta por
  defecto una función), `OOS2043` (la firma no se deriva) y `OOS2013` (el documento no es el que el
  código da), en ese orden; dos funciones con un nombre en un paquete, `OOS2035`.

`ore functions generate [--check] [--json]` lo hace en local; `ore-serve` lo hace al guardar.

### El contrato en ejecución

- **Python, `ore.contrato`:** `convertir` lleva cada parámetro al tipo que anota (`"2026-10-02"` es
  una `date`, `12.50` un `Decimal` exacto, un objeto una dataclass) y `comprobar_salida` comprueba lo
  devuelto; `ContractError` nombra el tipo anotado. `@function` envuelve el `def` con las dos.
- **Node, `contract` de `ore`:** Node **borra los tipos antes de ejecutar**, así que el contrato no
  puede leerlos del código: lee la **firma derivada** —la del documento, más una `forma` que el
  documento no dice: `BigInt` para un entero que el código declaró `bigint`—. `contract.call(fn,
  firma, args)` convierte por nombre (un decimal llega como la cadena de sus cifras; uno que un
  `number` no guarda tal cual, como el texto que se escribió), pone la fila delante con `over` y
  comprueba lo devuelto; `ContractError` dice el lado y el parámetro.

### Las operaciones

| operación | qué hace | dónde |
|---|---|---|
| **derivar** | la firma de un texto **sin guardar** (`.py` o `.ts`): las funciones en el orden del fichero, cada una con su firma (con su `runtime` y su `forma`) o sus fallos con línea | `POST /funciones/firma` (sólo lectura: no lee git ni ejecuta) |
| **probar** | Dry Run: la función del editor, con parámetros, en la sesión de la persona, sin commit | una celda del puesto (Python o Node) |
| **publicar** | guardar el `.py` o el `.ts` escribe su documento en el **mismo commit**; procedencia = ese commit | `PUT`/`POST /arbol…` (G2) |
| **describir** | superficie, código, corridas | `GET /funciones`, el índice de assets |
| **invocar** | `{parametros}` validados por su tipo antes de encolar → un trabajo del puesto con el arnés de su lenguaje → el informe en `resultados/`, en la rama de quien la invoca | `POST /funciones/{p}/{nombre}/invocar` |
| **llamar desde código** | `ore.get_function("p.def")(…)`: corre aquí, con su contrato (Python; sin `over` ni `models`) | el SDK |
| **versionar** | quitar un parámetro, mayor; uno obligatorio nuevo, mayor; uno opcional, menor | `ore diff` por parámetro |

Quién invoca: persona o agente, preguntado a `ore-acceso` (`puede`, 0047) antes de encolar;
`authorization` (Cedar) sigue en 422 hasta que se evalúe (0029 ④). Una función de TypeScript que
declara `models` es 422: el SDK de Node no llama a un modelo todavía, y no se finge.

### Las dependencias

Cada repositorio declara lo que su código importa donde su mundo lo declara —`pyproject.toml`,
`package.json`, `pom.xml`— y un Job con red lo resuelve en una **capa** del bucket que el puesto,
sin red, baja al arrancar. Cómo, con su lock, su grupo de desarrollo y lo que la sesión ya trae:
[El entorno de un repositorio · Las dependencias](#las-dependencias-1).

## Las superficies

### Code repositories

- **Las plantillas** siembran un repositorio entero alrededor de una función de verdad, la misma
  en los dos lenguajes (`InvoiceStatus`: lo pendiente, los días y el recargo si vence; `Decimal`,
  una fecha, un opcional), con un nombre único en el paquete sacado de la carpeta —
  `functions-python` v12 y `functions-typescript` v5, ver [Lo que nace](#lo-que-nace)—.
  **No siembran el documento:** lo escribe el primer commit, y la función nace en Dry Run y en
  Assets.
- **Dry Run**, tras el f(x) a la izquierda de *Results* en el panel de abajo, sólo en un repositorio
  de Functions:
  - **a la izquierda**, las funciones del fichero abierto, **en vivo**: tras una pausa del teclado
    el texto va a `POST /funciones/firma` y la lista se rehace (marca *LIVE*; *LAST COMMIT* si el
    servidor no la tiene). Una función que no se deriva sale con su fallo y *Line N*;
  - **a la derecha**, la tarjeta **Input**: un control por parámetro salido de su tipo de OOS
    —el decimal como texto exacto, `Money` con su moneda y sus decimales, `DateTimeTz` con su zona
    aparte, `Struct` con sus campos, `list` con *Add item*, `bytes` desde un fichero— o el JSON; lo
    rellenado se conserva mientras la firma cambia;
  - **Dry run** siempre se puede pulsar: con algo mal lleva al primer campo; con todo bien manda
    una celda a la sesión. En Python carga el borrador como módulo (sin su `__main__`); en
    TypeScript borra sus tipos (`stripTypeScriptTypes`, cada cosa en su línea), lo importa y llama a
    su `export default` con `contract.call` y la firma viva;
  - la tarjeta **Output**, sólo tras correr: **Success** o **Failed** con el tiempo. Un fallo dice
    **quién** no cumplió —la entrada, el código (con su línea), la salida, el fichero que no carga o
    la sesión—. Un éxito se pinta **por el tipo que la firma declara**: un **valor** (`36.18 EUR`,
    el instante con su zona y en UTC), un **registro** (campo → valor → tipo), una **tabla**
    (`list<Struct>`), **vacío** (`None`, `[]`) o **media** —un `Media<c>` con las celdas de la
    galería de Assets: la vista previa, su camino, su colección, con la URL firmada al pulsar—.
- **Functions en el Assets Catalog Explorer** del workspace: las publicadas, cada una abre su
  vista detallada.

### Assets → Functions

Una sección de primera clase, bajo Origins, sin base ni schema. La ficha (Overview, Metrics, Links,
History) y la **vista detallada**: *Signature*, *Code*, *Metrics* (las corridas, de **todas** las
ramas), *Configuration* (el `timeout`, de sólo lectura: está en el código), *History* y *Lineage*.
**No hay Invoke:** la acción principal es *Open in repository*. El icono es el f(x) del operador de
Pipelines.

### Pipelines

Por hacer: el operador `function` que llama a una función publicada por su nombre, con la fila.

## El entorno de un repositorio

Una función no vive sola: vive en un **repositorio**, y el repositorio es su entorno —lo que
declara, cómo se prueba, qué tipos ve el editor, con qué sesión corre—. Competir con Foundry
Functions no es sólo la firma: es que el repositorio funcione como uno de verdad, sin terminal y sin
instalar nada en el portátil. Esta es la imagen del de **Python**; el de TypeScript es el mismo con
sus herramientas, y la tabla del final los pone lado a lado.

```
 pyproject.toml ──commit──► ore-serve ── capas_tocadas ──► 52-la-capa (Job, con red)
   [project].dependencies      │  avisos de lo que no se lee      traer el árbol ─► capa-python:1 (uv, SIN testigo)
   [dependency-groups].dev     │                                    ─► capa.tgz · dev.tgz · informe · pylock.toml
                               │                                  subir-e-informar ─► bucket ore/puesto/<capa>/
                               │                                                  └─► commit: entorno/<capa>.json + pylock.toml
                               ▼
 GET /entorno/python ─► el panel Libraries: declarado · instalado · transitivas · lo de la sesión · avisos
                               │
 la sesión (puesto-python:1, Python 3.14) ── al arrancar: traer-la-capa ─► /capa (lo que corre)
   automática con un .py          └─ y sólo en sesión: /capa/.dev (el grupo dev) — una invocación nunca
   capa ≠ la del repositorio ─► se reinicia sola (functions, transforms) o pregunta (analytics, models)
   ore/probar ─► pytest en un proceso sin identidad ─► la lista, Expected/Actual y el Run log
   ore/comprobar ─► pyright sobre el repositorio tras un commit ─► lo roto en otros ficheros
```

### Lo que nace

`functions-python` **v12** siembra el repositorio entero; `functions-typescript` **v5**, su gemelo.

| fichero | Python | TypeScript |
|---|---|---|
| la declaración | `pyproject.toml`: `ore==1.0.0` en `dependencies`, `pytest==9.1.1` en el grupo `dev` | `package.json`: `ore` 1.0.0; `typescript` y `@types/node` en `devDependencies` |
| la función | `functions/example.py`, con su bloque de Run | `functions/<carpeta>InvoiceStatus.ts` |
| sus pruebas | `functions/test_example.py` (pytest, 5 casos) | `functions/<…>.test.ts` (`node:test`, 4) |
| `.gitignore` | `__pycache__/`, `.pytest_cache/`, `.venv/`… | `node_modules/`… |
| la guía | la prosa del manifiesto (README): estructura, tipos, configuración, probar, paquetes | ídem |
| lo demás | — | `tsconfig.json` (lo que Node corre: `erasableSyntaxOnly`) |

Lo declarado **es exactamente lo que trae la sesión**, a su versión —pruebas lo comparan con las
listas de la imagen—, así que no cuesta capa: el repositorio dice con qué trabaja, como un
repositorio de Foundry declara su `functions-api`. **Nace en verde:** una prueba siembra la plantilla
y le pasa pytest (5 passed), y pyright no marca nada en ella (la v12 corrigió dos errores de tipos de
la v11 en el bloque de Run).

### Las dependencias

| | |
|---|---|
| **dónde** | `[project].dependencies` (PEP 508) y el grupo `dev` de `[dependency-groups]` (PEP 735), como `devDependencies`. Nada más: `optional-dependencies`, otros grupos, Poetry, `[tool.uv]` y lo local no se leen, **y se dice** —en el panel y al commitear— |
| **cuándo** | un commit que toca el fichero **encola su capa** (`capas` en la respuesta); y al abrir una sesión con la capa pendiente |
| **quién resuelve** | `52-la-capa`, el gemelo de la de Node: el árbol con el testigo; **`capa-python:1` sin testigo ni credencial de nube** —uv 0.12.23 contra PyPI—; y quien sube y commitea, sin salir a PyPI |
| **con qué** | **lo que la sesión trae es una restricción** (`constraint-dependencies` con `puesto/python/provisto.txt`): lo compatible se resuelve, lo que choca es un error que lo dice |
| **cómo** | **sólo ruedas** (`no-build`): ningún paquete ejecuta código al instalarse. Una resolución, **dos cajas**: `capa.tgz` (lo que corre) y `dev.tgz` (el grupo `dev`), ninguna con lo de la sesión, cada una con su `sha256` |
| **el lock** | **`pylock.toml`** (PEP 751), commiteado junto al `pyproject.toml`: el estándar de Python, con lo de la sesión fijado a su versión —es la restricción con la que se resolvió—. Un commit que deja sólo lo de la sesión lo retira |
| **el nombre** | `capa-` + 12 hex del sha256 del **intérprete** (`cp314`) y lo declarado: cambiar de Python es cambiar de capa, y una caja de otro intérprete no llega a una sesión |
| **el SDK** | `ore` 1.0.0 (su `__version__`) es de la sesión: se declara, no pide capa y nunca se busca en PyPI —donde hay otro `ore`— |
| **por rama, por contenido** | la declaración y su lock son ficheros del repositorio: cada rama tiene los suyos y se fusionan con el código. La caja se nombra por su contenido: dos ramas —o dos repositorios— que declaran lo mismo comparten la capa |

### La sesión

- **`puesto-python:1`, Python 3.14** con pytest; el Job de la capa lee el intérprete de la imagen.
- **Automática:** un `.py` de functions o transforms conecta su sesión sin pulsar Run —con la pestaña
  visible, no si la desconectaste tú, un intento por minuto tras un fallo—, y la actividad del editor
  cuenta para su TTL. En analytics y models no: una exploración se abre a mano.
- **Baja su capa al arrancar** (`/capa`, y `/capa/.dev` sólo en una sesión: **lo de desarrollo nunca
  llega a una invocación**), detrás de la imagen en `sys.path`: manda el contenedor.
- **«Libraries changed».** La ficha del puesto dice con qué capa arrancó; si la del repositorio es
  otra y está lista —por Add, un commit a mano, otra pestaña—, en functions y transforms **se
  reinicia sola** en cuanto está ociosa; en analytics y models se pregunta.

### Las pruebas y los tipos

- **El repositorio en disco**, en la sesión: el de su rama, con el espejo de lo que el editor abre.
  Con él, pyright resuelve lo que un fichero importa de otro.
- **`ore/probar`:** pytest con un informe propio (una línea JSON por prueba, la forma del de Node) y
  la salida de la terminal como **Run log**: estado, línea, mensaje, **Expected/Actual** de un
  `assert a == b`, lo impreso. Todo el repositorio, un fichero o una prueba (con su caso
  parametrizado), con los borradores sin guardar encima. Un fichero que no carga no para el resto.
  **En un proceso sin la identidad de la sesión**: una prueba unitaria no lee datos. Topes: 30 s por
  prueba, 120 s en total.
- **`ore/comprobar`:** tras un commit, pyright sobre **el repositorio entero** tal como quedó: lo
  que un cambio rompe en un fichero que nadie tiene abierto.

### Libraries

El panel del sidebar, bajo Pull requests:

- **lo declarado** (Dependencies, Dev dependencies), con lo instalado de verdad (del lock) y sus
  transitivas; **lo que trae la sesión** en tarjetas (`session`), y en ámbar lo pedido a otra
  versión;
- **«+» → las sugeridas:** 30 por ecosistema, **medidas antes de entrar** —en Python, con el
  resolutor de la capa: las 30 resuelven contra la sesión en 3.14, la mayor 49 MB—, con la etiqueta
  `dev` las de probar. Un atajo, no un límite: cualquier paquete se declara a mano;
- **la ficha** de una librería, como una pestaña (el registro, sus versiones, su uso en el
  repositorio), y **Add**: la línea en el fichero —una **edición de texto** que no pisa comentarios
  ni orden— y un **commit directo**; *Installing…* hasta que la capa está lista; y la sesión, con
  ella.

### Upgrade fusiona

Actualizar un repositorio a la plantilla de hoy propone sus ficheros en una rama, como siempre; pero
**la declaración no se sustituye: se fusiona** (`ore-core/declaracion.rs`). Se añade lo que la
semilla declara y falta —por nombre, PEP 503 en Python— y no se toca lo que ya hay, con su versión;
el diff es exactamente lo añadido, y la propuesta lo dice. Lo mismo con `package.json` y
`.gitignore`.

### Python y TypeScript, lado a lado

| | Python | TypeScript |
|---|---|---|
| runtime | Python 3.14 (`puesto-python:1`) | Node 24 (`puesto-node:1`) |
| declarar | `pyproject.toml` · grupo `dev` | `package.json` · `devDependencies` |
| resolver | uv, sólo ruedas | npm, `--ignore-scripts` |
| lo de la sesión | **restricción**: lo que choca es un error | gana la de la sesión, con aviso |
| lock | `pylock.toml` (PEP 751) | `package-lock.json` |
| cajas | `capa.tgz` + `dev.tgz` | `capa.tgz` + `tipos.tgz` |
| pruebas | pytest, `test_*.py` | `node --test`, `*.test.ts` |
| tipos tras commit | pyright | `tsc` |
| editor | pyright con el repositorio en disco | typescript-language-server, ídem |

**Por qué Python resuelve con restricciones y Node avisa:** dos `numpy` no conviven en un intérprete
—un ABI que no casa no da una excepción, mata el proceso—, mientras que dos copias de un paquete de
npm son, como mucho, dos módulos.

## En vivo

| | |
|---|---|
| `victor` | `ore-serve` con G1–G5; la consola con Assets → Functions y Dry Run; `invoice_status` en Dry Run: *Success*, `0.04 ms`, el registro entero |
| TypeScript | en `victor`: un repositorio nuevo con su sesión automática, sus 4 pruebas en Test con el Run log, sus librerías con Add |
| el entorno de Python | en `victor` (P1–P5): Python 3.14, la capa con uv —`polars` en `/capa`, `hypothesis` en `/capa/.dev`, la misma caja compartida por dos repositorios que declaran lo mismo—, `pylock.toml` en el repositorio, pytest y pyright en la sesión |
| las pruebas | `el-puesto.sh` 18; `los-documentos.sh` 25; conformidad v1alpha18, v1alpha20 y **v1alpha23 (32 casos)**; el oráculo y las instantáneas de `ore-code` y 17 pruebas del lector de TypeScript con los hostiles; 18 del contrato de Python y 7 del de Node; **29 del repositorio de Python** (pytest, pyright de verdad, `materializar` contra un `ore-serve` de mentira) y «nace en verde»; la fusión (8) y Add (6) con los mismos casos; las imágenes `capa-node`, `capa-python` y `capa-jvm` se prueban al construirse |
| **medido** | una invocación bajo demanda tarda ~68 s en `victor` (42 esperando nodo): es un trabajo; Dry Run en una sesión viva, milisegundos; derivar una firma, microsegundos. TypeScript, en local con Node: la celda de Dry Run y el arnés (ok, entrada, código con su línea, carga), un entero de 2⁵³+1 y un decimal largo exactos; la capa de npm, la misma caja dos veces |

## De dónde viene

El 2026-10-01 el producto se reorientó para competir con Foundry Functions sobre lo que ORE ya
define. El primer marco (P1–P6) pedía escribir el contrato en YAML junto al código; **la
corrección de ese mismo día** fue la que lo ordenó todo: *nadie escribe la función en YAML*. Desde
ahí:

| | qué | commits |
|---|---|---|
| P1–P5 | v1alpha18; `ore-core` la habla y `ore diff` ve cada parámetro; invocar con el arnés; `ore.modelo()` y su red; la plantilla | `4081ef9`…`1882f89` |
| G0–G1 | el documento se deriva: OOS §4; `ore-code`; el cotejo en el compilador; `ore functions generate` | `362ed2c`, `8fc664f`, `9c1cd1d`, `4a89e70` |
| — | el paquete de un proyecto puede publicar: su id es un identificador (`ore migrate proyectos`) | `601ead3` |
| G2 | guardar un `@function` escribe su documento en el mismo commit | `ac818af` |
| — | una función publicada vive en `functions/` del paquete | `389ee4c` |
| F1–F4 | el activo de función en el índice; Assets → Functions; las corridas de todas las ramas | `b4aeeca`, `bc5d865` |
| G3 | el contrato se cumple: en la sesión, al invocarla, desde código | `6898593` |
| G4 | la plantilla v8 | `2a718c4` |
| G5a | la firma habla OOS (v1alpha20) | `5ec35ba` |
| G5b | Dry Run: la firma en vivo, correr en la sesión, Output por tipo, Media | `996f6d7` + consola |
| **R3 · T0** | OOS v1alpha23: `runtime: node`, la marca, los tipos, 32 casos | oos `f29ea90`, `c58acab` |
| T1 | el lector de TypeScript en `ore-code` (oxc) y `ore-core` para `node`; conformidad 32/32 | `ddcd8b7` |
| T2 | el SDK de Node: los tipos (`index.d.ts`) y el contrato; `Tipo::Bigint` y su `forma` | `0774817` |
| T3 | `ore-serve`: la firma en vivo de un `.ts` y su documento al guardar | `3a0e5c5` |
| T4 | Dry Run de TypeScript: la firma viva y la celda de Node | consola `7758843` |
| T5 | invocar `runtime: node`: el arnés de Node, como trabajo del puesto | `0d90fd4` |
| T5b | la capa de Node: `package.json`, `capa-node:1`, `57-la-capa-node.yaml` | `34e8221` |
| T6 | la plantilla `functions-typescript` v3: una función de verdad | `1ea43e8` |
| **L1–L2** | TypeScript, el repositorio entero: la plantilla v4 (pruebas, `tsconfig`, `.gitignore`, la guía); el lock al repositorio | `9ada13b`, `b5ab90b` |
| L3 | una resolución, dos cajas (`devDependencies`); la correa de Node: el servidor de TypeScript con el repositorio en disco | `fe75669`, `2f44392` |
| L4–L5 | `ore/comprobar` (`tsc`) y `ore/probar` (`node --test`) en la sesión | `5a0994d`, `ba12221` |
| L6 | Libraries: lo que trae cada sesión, lo de la imagen no pide capa, la plantilla v5; `ore-packages` (la ficha y la búsqueda en npm, PyPI y Maven); las sugeridas; el commit que toca la declaración encola la capa | `089d6e5`…`8aa2097`, `a7a352f`, `42fe14b`, `8a00ec6`, `de95074` |
| — | la sesión automática de TypeScript; el Run log de Test | `b82ee0e`, `91f4293` |
| **P0–P1** | medido: Python 3.14 sin coste; `puesto-python` en 3.14 con pytest, el intérprete en el nombre de la capa | `daea844` |
| P2 | Python declara como Node: el grupo `dev` y `pylock.toml`; `capa-python:1` con uv; `52-la-capa` en tres contenedores con dos cajas | `5f38282`, `1222307`, `8df8039`, `25d4a1f` |
| P3 | el SDK con versión; `functions-python` v11, el repositorio entero; Upgrade fusiona | `e99fb5b`, `251f8b4`, `ed4cebd`, `0b141b7` |
| P4 | Test y comprobar en Python: el repositorio en disco, pytest y pyright; la plantilla v12 | `f3f3e37`, `092019e` |
| P5 | «Libraries changed» y el reinicio solo; la sesión automática de Python; las 30 sugeridas; Add a `pyproject.toml`; lo que no se lee, dicho; los avisos en inglés | `73164a0`, `2ae4bb8`, `7df281c`, `9f31d7a` + consola |

## Las decisiones

| # | decisión | |
|---|---|---|
| A1 | el producto es *Code Repositories · Functions*; este documento es su marco | 2026-10-01 |
| A2 | Python es el fundamento; TypeScript, Node y Java después, por la misma regla | 2026-10-01 |
| A3 | la escritura (acciones) tiene su propia especificación | 2026-10-01 |
| A4 | la base es lo que ORE ya define; sin piezas paralelas | 2026-10-01 |
| D1 | el código manda: la marca y el documento **derivado** de la firma, sin ejecutar; `OOS2013` si difieren | 2026-10-01 |
| D2 | sin `apiName`: el nombre es el del catálogo, `<paquete>.<nombre>` | 2026-10-01 |
| D3 | la enmienda a 0029 de 0031 W3.8: el aislamiento de una función de código es la red cerrada; el contenedor del cliente, fuera | 2026-10-01 |
| D4 | el plazo es `limits.timeout`, de `@function(timeout=…)` o de `config` | 2026-10-01 |
| D5 | la versión es la del paquete, por `ore diff` | 2026-10-01 |
| D6 | la red, por política con nombre seleccionada por lo declarado | 2026-10-01 |
| D7 | una función publicada vive en `packages/<p>/functions/<nombre>.yaml`; dueño implícito, el paquete del repositorio | 2026-10-01 |
| D8 | Assets observa y configura; no ejecuta (sin Invoke) | 2026-10-02 |
| D9 | la firma habla los tipos de OOS (v1alpha20); el documento declara la versión más baja que lo deriva | 2026-10-02 |
| D10 | probar se llama **Dry Run**: en vivo, sin commit, en la sesión, con el contrato | 2026-10-02 |
| D11 | Output es nuestro: dos estados y el resultado por su tipo, sin pestañas Result/Logs/Performance | 2026-10-03 |
| D12 | la marca de TypeScript es el estándar —`export default function` en un `.ts` de `functions/`, que se llama como el fichero, y `export const config`—, no un decorador | 2026-10-03 |
| D13 | en TypeScript un `number` es `Float`; un entero es `Integer` o `bigint`; un decimal, una cadena con sus cifras; un `Date`, un instante; las fechas de calendario, cadenas ISO | 2026-10-03 |
| D14 | el parser de TypeScript es oxc, fijado; sin el compilador de TypeScript: la firma es lo escrito | 2026-10-03 |
| D15 | el contrato de Node lee la firma **derivada** (con su `forma`), porque Node borra los tipos | 2026-10-03 |
| D16 | la capa de npm: `--ignore-scripts`, rangos con la resolución exacta en el lock, manda el contenedor; el código y la imagen antes que la malla en un inquilino | 2026-10-03 |
| D17 | **el repositorio es el entorno**: nace entero (declaración, función, pruebas, `.gitignore`, guía) y declara su SDK y sus herramientas a la versión de la sesión, sin costar capa | 2026-10-03 |
| D18 | el lock se commitea **junto a su declaración**, en el formato estándar de cada mundo (`package-lock.json`, `pylock.toml`), y lo escribe la plataforma | 2026-10-03 |
| D19 | lo de desarrollo en **su propia caja**: llega a la sesión (pruebas, editor) y nunca a una invocación | 2026-10-03 |
| D20 | pruebas y tipos **en la sesión**, sobre el repositorio en disco; las pruebas, en un proceso sin la identidad de la sesión | 2026-10-03 |
| D21 | sin terminal: Libraries (sugeridas medidas, ficha, Add con commit directo) y el commit que toca la declaración, que la instala | 2026-10-04 |
| D22 | la sesión de un repositorio se conecta sola con un fichero suyo (functions, transforms) | 2026-10-05 |
| D23 | Python 3.14; el intérprete entra en el nombre de la capa | 2026-10-05 |
| D24 | Python: el grupo `dev` de PEP 735 y `pylock.toml` de PEP 751; uv 0.12.23 fijado; sólo ruedas | 2026-10-05 |
| D25 | en Python **lo de la sesión es una restricción**, no un aviso | 2026-10-05 |
| D26 | quien resuelve una capa (`capa-python`, `capa-node`, `capa-jvm`) no lleva el testigo ni credencial de nube | 2026-10-05 |
| D27 | el SDK de Python tiene versión (`ore` 1.0.0) y es de la sesión: se declara y nunca se busca en un registro | 2026-10-05 |
| D28 | Upgrade **fusiona** la declaración: añade lo que falta y no toca lo que hay | 2026-10-05 |
| D29 | una sesión con una capa vieja se reinicia sola donde no guarda nada de valor (functions, transforms) y pregunta donde sí (analytics, models) | 2026-10-05 |
| D30 | lo que se declara donde no se lee, se dice; y lo que la consola enseña, en inglés | 2026-10-05 |

## Lo que se decidió no hacer

- **El contrato escrito a mano como camino principal.** Sigue valiendo sin marca, pero el producto
  es el código.
- **Invoke en Assets.** Ejecutar es de donde se usa; Assets es observabilidad y configuración, como
  el Ontology Manager de Foundry.
- **Un registro de funciones fuera del árbol, `apiName`, versiones por función** (Foundry): aquí el
  árbol es el registro y la versión es la del paquete.
- **El contenedor del cliente** (*compute modules*): 0029 lo rechaza.
- **Las pestañas Result/Logs/Performance** de Foundry en Dry Run: la pregunta es «¿cumple su
  contrato?», y se contesta con un veredicto y el valor.
- **Decoradores en TypeScript** (las clases de Foundry TS v1) y **una envoltura propia** (`fn(…)`):
  el estándar de la industria ya es una marca explícita que se lee sin ejecutar.
- **Inferir la firma con el compilador de TypeScript:** sería otra respuesta en cada versión del
  compilador y una dependencia enorme; la firma explícita es lo que un consumidor ve.
- **Una terminal en la sesión** (la tiene el Code Repositories de Foundry): por ahora, lo que una
  terminal haría aquí —instalar, probar, comprobar tipos— tiene su camino propio, declarado y con su
  commit. Sigue propuesta.
- **Instalar en caliente** en una sesión viva: sólo valdría para lo que aún no se importó, y fallaría
  en silencio al cambiar la versión de algo ya cargado. Se reinicia: unos segundos.
- **En Python, ganar la sesión con un aviso** (como en Node): un aviso que nadie lee acaba en un
  intérprete que muere.
- **`uv.lock` en el repositorio:** es de uv; `pylock.toml` es el estándar que cualquier herramienta lee.
- **Un entorno por repositorio y no por rama:** cambiar una librería en una rama cambiaría `main` sin
  propuesta, y el lock dejaría de corresponder al código.
- **Sugerir lo que no funciona en el puesto:** xgboost (380 MB de CUDA sin GPU), email-validator
  (DNS, sin red), y en Node lo que baja binarios o modelos al instalarse.

## Lo que sigue

| | qué |
|---|---|
| **TypeScript en vivo** | la imagen con R3 y `capa-node:1`, converger los inquilinos, y probarlo en `victor`: crear el repositorio, Dry Run, invocar |
| **Pipelines** | el operador `function`: una función publicada como paso, con la fila |
| **Dry Run** | las funciones `over` (una tabla de filas); en TypeScript, un `import` relativo (`./util.ts`) del borrador |
| **TypeScript** | `models` desde Node; un tipo de otro fichero; pedir la capa de npm desde la consola, como la de la JVM; `jvm` por la misma regla |
| **G2c** | el panel de Commit marca los documentos que escribe la plataforma |
| **la firma** | una `Entity` como parámetro; `set`/`dict`; uniones de literales; `Vector`/`Anchor` |
| **Configuration** | sobrescribir el `timeout` sin tocar el código, si se pide |
| **R2** | residente —o *isolates* para lo puro—, cuando la latencia medida lo pida |
| **R4** | aplicaciones externas con su cliente (0048) |
| **la red** | `salida-al-modelo-de-una-funcion` llega a cada inquilino al converger su malla |
| **Libraries** | *Change version* y *Remove* desde la ficha (hoy, a mano en el fichero) |
| **la capa** | una rama nueva sin el informe de la capa de `main` repite el Job aunque la caja ya esté (el informe se busca en el árbol, no en el bucket) |
| **el relevo** | un `503` del proxy de la malla durante el relevo `Recreate` de `ore-serve`: la consola debería reintentar |
| **Java** | el entorno de un repositorio de Java por la misma regla: su SDK no es un paquete de Maven (un nombre reservado, como `ore` en Python), sus sugeridas, sus pruebas y la sesión automática |

## Qué se acepta a cambio

- **Dos lectores propios** (`ore-code`, con Ruff y con oxc) y `stacker`/`psm` enlazados en `ore`: el
  cierre de `ore-cli` pasó de 34 a 126 crates con Python (G1c) y a 183 con TypeScript (R3 T1; 85 →
  143 enlazados, sin nada vetado; `textwrap` e `icu_segmenter` vienen con `oxc_diagnostics` y no se
  apagan).
- **~70 s por invocación** mientras sólo haya bajo demanda; Dry Run es lo rápido.
- **El aislamiento de una función de código es la red cerrada,** no WASI (0031 W3.8).
- **En TypeScript, la firma se escribe entera** —la vuelta incluida— y sólo con lo del mismo fichero.
- **Un paquete de npm que compila código nativo al instalarse no funciona** en la capa
  (`--ignore-scripts`); el informe lo dice. **En Python, uno que sólo publica fuentes, tampoco**
  (`no-build`).
- **Las capas de Python se resolvieron otra vez** al pasar a 3.14 (el intérprete está en su nombre):
  una vez por repositorio, al abrirse.
- **La sesión trae la última palabra:** pedir otra versión de pandas, numpy o pyarrow es un error en
  Python; para cambiarlas hay que cambiar la imagen, para todos los repositorios a la vez.
- **Un reinicio de la sesión** cuando cambian sus librerías: en functions y transforms, solo y en
  unos segundos; en analytics, se pierde lo que había en sus variables si se acepta.

## Anexo · Foundry, lo que se tomó y lo que no

De `palantir.com/docs/foundry`, leído el 2026-10-01 (Python) y el 2026-10-03 (TypeScript v2).
**Se toma:** la firma tipada como centro, el formulario generado de la firma para probar sin
publicar, la lista de lo incompatible entre versiones, los recursos declarados que abren la red, la
observabilidad sin ejecución en el Ontology Manager y, en TypeScript, la marca de v2 —un fichero por
función, su `export default` y un `config` al lado—. **No se toma:** el registro aparte, el
`apiName`, las versiones por función, el contenedor del cliente, la salida en tres pestañas y los
decoradores de clase de v1. **Va más lejos:** los tipos del catálogo en la firma (`Money`,
`DateTimeTz`, `Struct`, `Media`) —en TypeScript también `Decimal`, que Foundry no tiene, y un entero
que no pierde cifras—, la firma en vivo mientras se escribe, el fallo que dice quién no cumplió, y
las ramas: Foundry TypeScript v2 sólo compila contra main, y aquí una función se escribe, se prueba
y se invoca en la rama de quien la toca.

**El repositorio** (2026-10-05). Foundry siembra el suyo con su SDK y sus herramientas declarados,
un lock de miles de líneas —jest, Babel, su generador de código, en `devDependencies`— y una
terminal. **Se toma:** declarar el SDK y las herramientas, el lock en el repositorio, las pruebas
junto al código, un panel de librerías. **No se toma:** la terminal (por ahora) y la cadena de
herramientas en cada lock: aquí el runner, el comprobador de tipos y el SDK viven en la imagen de la
sesión —Node 24 trae `node:test` y borra los tipos; Python 3.14 trae pytest y pyright—, y el lock
de un repositorio es sólo lo suyo. **Va más lejos:** las sugeridas medidas contra la sesión, lo de
la sesión como restricción en Python, Upgrade que fusiona y la sesión que se reinicia sola con sus
librerías nuevas.
