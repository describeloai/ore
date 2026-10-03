# 0050 · Functions in code repositories

**Estado:** aceptado y **en vivo** en `victor` desde el 2026-10-03: R1 completo, de punta a punta
—escribir el `def`, probarlo en Dry Run sin commit, publicarlo al commitear, verlo en Assets e
invocarlo—. Abiertos: el operador `function` de Pipelines, Media en Dry Run, las funciones `over`
en Dry Run y lo que se lista al final.

Construye sobre lo que ORE ya define —la `Function` de OOS (v1alpha10–14), el puesto
([`0031`](0031-el-puesto.md)), la versión del paquete (`91-versioning`), la red por política con
nombre (0031 §7) y el acceso ([`0047`](0047-ore-access-control.md))— y **ejecuta** la enmienda a
[`0029`](0029-donde-corre-una-funcion.md) que 0031 W3.8 dejó escrita.

## Qué es

**Una función es un `def` de Python con anotaciones de tipo y `@function`.** La persona escribe
sólo eso. La plataforma **deriva** de la firma el documento `Function` —el contrato publicado—, lo
escribe en el mismo commit, lo enseña en Assets y lo hace cumplir igual en todos los sitios donde
la función corre.

```python
from dataclasses import dataclass
from decimal import Decimal
from ore import function
from ore.tipos import DateTimeTz, Money

@dataclass
class Quote:
    total: Money["EUR", 2]
    valid_until: DateTimeTz

@function(timeout="30s")
def quote_order(order: Order, cutoff: DateTimeTz, discount: Decimal = Decimal("0")) -> Quote:
    """Quote an order: its total in EUR and until when the quote holds."""
    ...
```

```
 editor ──texto──► POST /funciones/firma ──► la firma, en vivo ──► Dry Run (formulario por tipo)
   │                    (ore-code)                                   │ Dry run: una celda en
   │                                                                 ▼ la sesión, con el contrato
   └─commit──► ore-serve deriva y escribe packages/<p>/functions/<def>.yaml en el MISMO commit
                    │
                    ├─► Assets → Functions: observa y configura (no ejecuta)
                    └─► se invoca: POST …/invocar (un trabajo del puesto) · ore.funcion() desde código
```

### Los principios

1. **El código manda; el documento se deriva.** Nadie escribe la función en YAML. El `Function`
   sale de la firma **leyendo el fichero, sin ejecutarlo** (`ore-code`), y el compilador lo coteja:
   `OOS2013` si el documento no es el que el código da. Uno escrito a mano, sin `@function`, sigue
   valiendo y no se toca.
2. **La firma habla OOS.** Los tipos de los parámetros y de la vuelta son los del catálogo
   —`Money<EUR, 2>`, `DateTimeTz`, `Struct`…—, no los de Python. Lo que no se traduce no se deriva
   (`OOS2043`), con su ayuda.
3. **El contrato se cumple igual en todas partes.** La misma pieza (`ore.contrato`) convierte la
   entrada y comprueba la salida en Dry Run, en la sesión, al invocarla y al llamarla desde código:
   lo que funciona en un sitio funciona en los demás.
4. **Una función publicada es un nombre del paquete,** `<paquete>.<def>`, sin base ni schema que
   elegir: vive en `packages/<paquete>/functions/<def>.yaml`, nunca junto al código.
5. **Se ejecuta en el puesto, en la celda;** nunca en `ore-serve` (0029 ①).
6. **Lo publicado tiene versión, y la decide `ore diff`,** la del paquete; no hay versiones por
   función.
7. **Sólo se alcanza lo declarado:** `over`/`reads` para los datos, `models` para los modelos; la
   red se abre por política con nombre.
8. **Assets observa y configura; no ejecuta.** Una función se ejecuta desde donde se usa —Pipelines,
   el código—, y se prueba donde se escribe (Dry Run).

## El contrato

### El documento

La `Function` de OOS. `input` y `output` **son** la firma; lo que v1alpha18 añadió:

| | qué | por qué |
|---|---|---|
| `runtime: python` | el código es un `def` de Python | la promoción de 0031 W3.8; `node` y `jvm`, por la misma regla |
| `entrypoint: <ruta>.py:<def>` | desde la carpeta del paquete | lo que `wasm` ya exigía, con el `def` nombrado |
| `models: [modelo/<ref>]` | los modelos que el código **puede** llamar | `model` sigue siendo `runtime: model`; una función de código **usa** modelos, y lo usado se declara |
| `output` como valor | `{type: T}` si la vuelta no es una `@dataclass` | una función puede devolver un escalar o una lista |
| `limits.timeout`, `over`, `reads`, `models` | argumentos **literales** de `@function(...)` | se leen sin ejecutar |
| `owner` (v1alpha21) | quién responde de la función: **quien la crea**, `user:<handle>` | no sale del código, así que `OOS2013` no lo compara; al regenerar el documento se conserva (0052 · Ownership) |

El documento derivado declara **la versión más baja que lo describe**: v1alpha18; v1alpha20 si la
firma usa sus tipos; **v1alpha21 si lleva `owner`**, que es lo que pasa cuando nace desde la
plataforma —al guardar el código o al sembrar un repositorio—, porque el servidor sabe quién lo
crea. Generado en local, sin plataforma, no lleva `owner` y sus bytes son los de siempre: un árbol
que no usa nada nuevo no cambia ni un byte.

### Los tipos de la firma

| Python | OOS | desde |
|---|---|---|
| `int` · `float` · `str` · `bool` · `date` · `datetime` · `Decimal` | `Integer` · `Float` · `String` · `Boolean` · `Date` · `DateTime` · `Decimal` | v1alpha18 |
| `list[T]` · `T \| None` | `list<T>` · no requerido | v1alpha18 |
| `time` · `bytes` | `Time` · `Opaque` (viaja en base64) | v1alpha20 |
| `ore.tipos.DateTimeTz` | `DateTimeTz` (un instante sin zona es un error) | v1alpha20 |
| `Annotated[Decimal, Precision(p, s)]` | `Decimal<p, s>` | v1alpha20 |
| `Money["EUR", 2]` · `Quantity["km", 1]` | `Money<EUR, 2>` · `Quantity<km, 1>` | v1alpha20 |
| una `@dataclass` del fichero · `list[D]` | `Struct<…>` · `list<Struct<…>>` (una que se contiene a sí misma, `OOS2043`) | v1alpha20 |
| `Media["base.schema.coleccion"]` | `Media<…>`: la referencia a un ítem; la colección tiene que existir (`OOS2018`) | v1alpha20 |

`ore.tipos` son **anotaciones**: un `Annotated` de lo que Python ya tiene, con argumentos literales.
Fuera por ahora: una `Entity` como parámetro, `set`/`dict`, `Vector`/`Anchor`, identidad y marcas.

### La derivación: `ore-code`

Un crate propio, el único sitio que entiende Python:

- **Analiza** con `ruff_python_parser` (fijado; 655/655 ficheros de la stdlib de 3.14) y **resuelve
  los nombres como Python** sin ejecutar: alias, sombras, comillas, `from __future__ import
  annotations`, la evaluación de 3.12. La versión del puesto (3.12) se señala si el código usa algo
  posterior.
- **Nunca se para en el primer problema:** sintaxis, tipos sin traducción y argumentos no literales
  son datos, todos a la vez, con su sitio en el fuente.
- **Es hostil-seguro:** pila dimensionada por byte y 1 MiB de límite.
- **Tiene un oráculo:** `tests/oraculo.py` escribe la misma regla sobre el `ast` de CPython, y un
  corpus diferencial exige que los dos deriven lo mismo.
- Los fallos: `OOS2042` (el `entrypoint` no nombra un `def` síncrono), `OOS2043` (la firma no se
  deriva) y `OOS2013` (el documento no es el que el código da), en ese orden.

`ore functions generate [--check] [--json]` lo hace en local; `ore-serve` lo hace al guardar.

### El contrato en ejecución: `ore.contrato`

Del SDK. `convertir` lleva cada parámetro al tipo que anota (`"2026-10-02"` es una `date`, `12.50`
un `Decimal` exacto, un objeto una dataclass) y `comprobar_salida` comprueba lo devuelto; los
errores nombran el tipo anotado (`Money<EUR, 2>`, no `Decimal`). `@function` envuelve el `def` con
las dos, así que **en la sesión una función se llama con su contrato**, y el arnés de la invocación,
`ore.funcion("p.def")` y Dry Run usan las mismas.

### Las operaciones

| operación | qué hace | dónde |
|---|---|---|
| **derivar** | la firma de un texto **sin guardar**: las funciones en el orden del fichero, cada una con su firma o sus fallos con línea | `POST /funciones/firma` (sólo lectura: no lee git ni ejecuta) |
| **probar** | Dry Run: la función del editor, con parámetros, en la sesión de la persona, sin commit | una celda del puesto |
| **publicar** | guardar el `.py` escribe su documento en el **mismo commit**; procedencia = ese commit | `PUT`/`POST /arbol…` (G2) |
| **describir** | superficie, código, corridas | `GET /funciones`, el índice de assets |
| **invocar** | `{parametros}` validados por su tipo antes de encolar → un trabajo del puesto con el arnés → el informe en `resultados/`, en la rama de quien la invoca | `POST /funciones/{p}/{def}/invocar` |
| **llamar desde código** | `ore.funcion("p.def")(…)`: corre aquí, con su contrato (sin `over` ni `models`) | el SDK |
| **versionar** | quitar un parámetro, mayor; uno obligatorio nuevo, mayor; uno opcional, menor | `ore diff` por parámetro |

Quién invoca: persona o agente, preguntado a `ore-acceso` (`puede`, 0047) antes de encolar;
`authorization` (Cedar) sigue en 422 hasta que se evalúe (0029 ④).

## Las superficies

### Code repositories

- **La plantilla** `functions-python` (v8) siembra `pyproject.toml` y `funciones/ejemplo.py` con
  `<carpeta>_invoice_status`: un ejemplo de verdad —`Decimal`, `date`, una `@dataclass` de vuelta—,
  con nombres en inglés y un bloque `__main__` para Run. **No siembra el documento:** lo escribe el
  primer commit.
- **Dry Run**, tras el f(x) a la izquierda de *Results* en el panel de abajo, sólo en un repositorio
  de Functions:
  - **a la izquierda**, los `@function` del fichero abierto, **en vivo**: tras una pausa del teclado
    el texto va a `POST /funciones/firma` y la lista se rehace (marca *LIVE*; *LAST COMMIT* si el
    servidor no la tiene). Una función que no se deriva sale con su fallo y *Line N*;
  - **a la derecha**, la tarjeta **Input**: un control por parámetro salido de su tipo de OOS
    —el decimal como texto exacto, `Money` con su moneda y sus decimales, `DateTimeTz` con su zona
    aparte, `Struct` con sus campos, `list` con *Add item*, `bytes` desde un fichero— o el JSON; lo
    rellenado se conserva mientras la firma cambia;
  - **Dry run** siempre se puede pulsar: con algo mal lleva al primer campo; con todo bien manda
    una celda a la sesión que carga el borrador como módulo (sin su `__main__`), convierte la
    entrada, llama al `def` y comprueba la salida;
  - la tarjeta **Output**, sólo tras correr: **Success** o **Failed** con el tiempo. Un fallo dice
    **quién** no cumplió —la entrada, el código (con su línea), la salida, el fichero que no carga o
    la sesión—. Un éxito se pinta **por el tipo que la firma declara**: un **valor** (`36.18 EUR`,
    el instante con su zona y en UTC), un **registro** (campo → valor → tipo), una **tabla**
    (`list<Struct>`) o **vacío** (`None`, `[]`, como resultados válidos).
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

## En vivo

| | |
|---|---|
| `victor` | `ore-serve` con G1–G5; la consola con Assets → Functions y Dry Run; `invoice_status` en Dry Run: *Success*, `0.04 ms`, el registro entero |
| las pruebas | `el-puesto.sh` 18 (la función con el agente de verdad), `los-documentos.sh` 25, conformidad v1alpha18 y v1alpha20 (7 casos), el oráculo y las instantáneas de `ore-code`, 18 pruebas del contrato |
| **medido** | una invocación bajo demanda tarda ~68 s en `victor` (42 esperando nodo): es un trabajo; Dry Run en una sesión viva, milisegundos; derivar una firma, microsegundos |

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
| G5b | Dry Run: la firma en vivo, correr en la sesión, Output por tipo | `996f6d7` + consola |

## Las decisiones

| # | decisión | |
|---|---|---|
| A1 | el producto es *Code Repositories · Functions*; este documento es su marco | 2026-10-01 |
| A2 | Python es el fundamento; TypeScript, Node y Java después, por la misma regla | 2026-10-01 |
| A3 | la escritura (acciones) tiene su propia especificación | 2026-10-01 |
| A4 | la base es lo que ORE ya define; sin piezas paralelas | 2026-10-01 |
| D1 | el código manda: `@function` y el documento **derivado** de la firma, sin ejecutar; `OOS2013` si difieren | 2026-10-01 |
| D2 | sin `apiName`: el nombre es el del catálogo, `<paquete>.<def>` | 2026-10-01 |
| D3 | la enmienda a 0029 de 0031 W3.8: el aislamiento de una función de código es la red cerrada; el contenedor del cliente, fuera | 2026-10-01 |
| D4 | el plazo es `limits.timeout`, de `@function(timeout=…)` | 2026-10-01 |
| D5 | la versión es la del paquete, por `ore diff` | 2026-10-01 |
| D6 | la red, por política con nombre seleccionada por lo declarado | 2026-10-01 |
| D7 | una función publicada vive en `packages/<p>/functions/<def>.yaml`; dueño implícito, el paquete del repositorio | 2026-10-01 |
| D8 | Assets observa y configura; no ejecuta (sin Invoke) | 2026-10-02 |
| D9 | la firma habla los tipos de OOS (v1alpha20); el documento declara la versión más baja que lo deriva | 2026-10-02 |
| D10 | probar se llama **Dry Run**: en vivo, sin commit, en la sesión, con el contrato | 2026-10-02 |
| D11 | Output es nuestro: dos estados y el resultado por su tipo, sin pestañas Result/Logs/Performance | 2026-10-03 |

## Lo que se decidió no hacer

- **El contrato escrito a mano como camino principal.** Sigue valiendo sin `@function`, pero el
  producto es el `def`.
- **Invoke en Assets.** Ejecutar es de donde se usa; Assets es observabilidad y configuración, como
  el Ontology Manager de Foundry.
- **Un registro de funciones fuera del árbol, `apiName`, versiones por función** (Foundry): aquí el
  árbol es el registro y la versión es la del paquete.
- **El contenedor del cliente** (*compute modules*): 0029 lo rechaza.
- **Las pestañas Result/Logs/Performance** de Foundry en Dry Run: la pregunta es «¿cumple su
  contrato?», y se contesta con un veredicto y el valor.

## Lo que sigue

| | qué |
|---|---|
| **Pipelines** | el operador `function`: una función publicada como paso, con la fila |
| **Dry Run** | Media en Output (la vista previa del ítem); las funciones `over` (una tabla de filas); TypeScript (llega con R3) |
| **G2c** | el panel de Commit marca los documentos que escribe la plataforma |
| **la firma** | una `Entity` como parámetro; `set`/`dict`; `Vector`/`Anchor` |
| **Configuration** | sobrescribir el `timeout` sin tocar el código, si se pide |
| **R2** | residente, cuando la latencia medida lo pida |
| **R3** | `node`/`jvm` y la firma de TypeScript |
| **R4** | aplicaciones externas con su cliente (0048) |
| **la red** | `salida-al-modelo-de-una-funcion` llega a cada inquilino al converger su malla |

## Qué se acepta a cambio

- **Un lector de Python propio** (`ore-code`, con Ruff) y `stacker`/`psm` enlazados en `ore`: el
  cierre de `ore-cli` pasó de 34 a 126 crates (0050 G1c).
- **~70 s por invocación** mientras sólo haya bajo demanda; Dry Run es lo rápido.
- **El aislamiento de una función de código es la red cerrada,** no WASI (0031 W3.8).

## Anexo · Foundry, lo que se tomó y lo que no

De `palantir.com/docs/foundry`, leído el 2026-10-01. **Se toma:** la firma tipada como centro, el
formulario generado de la firma para probar sin publicar, la lista de lo incompatible entre
versiones, los recursos declarados que abren la red y la observabilidad sin ejecución en el
Ontology Manager. **No se toma:** el registro aparte, el `apiName`, las versiones por función, el
contenedor del cliente y la salida en tres pestañas. **Va más lejos:** los tipos del catálogo en
la firma (`Money`, `DateTimeTz`, `Struct`, `Media`), la firma en vivo mientras se escribe y el fallo
que dice quién no cumplió.
