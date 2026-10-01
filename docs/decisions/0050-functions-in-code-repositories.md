# 0050 · Functions in code repositories

**Estado:** propuesto · escrito el 2026-10-01 con lo de hoy medido (M1–M3) y el estado del arte de
Foundry recogido; las decisiones aprobadas y las pendientes, en [su tabla](#las-decisiones); R1–R6,
por construir. **Decide:** qué es el **producto** *Functions* de los code repositories —una función
es **una firma tipada** cuyo código es la fuente, que se ejecuta por **un solo contrato** en varios
modos de despliegue, se publica **por versión**, se invoca **con parámetros desde dentro y desde
fuera** y solo alcanza **lo que declara**— y el orden en que se construye. **Enmienda**
[`0029`](0029-donde-corre-una-funcion.md) en un punto: `runtime: wasm` deja de ser «la única forma de
traer código»; Python y Node corren en el puesto con la red cerrada y abierta solo por lo declarado
(⑤), y wasm queda como el modo de aislamiento estricto. **Mantiene** lo demás de 0029 (corre en la
celda, nunca en `ore-serve`) y la escritura de [`functions.md`](../functions.md) (una función no
escribe, propone), que tiene su propia especificación.

## La pregunta

¿Qué tiene que ser verdad para que un repositorio de clase *functions* sea **el mejor sitio para
construir lógica encapsulada** sobre los conjuntos de datos del catálogo y sobre los modelos —el
culmen de un IDE para eso— y para que encima se puedan construir **niveles de abstracción** después
(módulos de cómputo consultables desde fuera, acciones, lógica sin código, aplicaciones) **sin volver
a escalar la infraestructura de base**?

La respuesta de este documento es **cinco piezas** que hay que fijar ahora, porque cambiarlas
después rompe a todos los que ya dependan de ellas, y **una primera rebanada** (R1) que cierra la
brecha de hoy dentro de ese marco y no fuera de él.

## Cómo se lee

De lo más abstracto a lo más concreto, como [`0049`](0049-media-paradigms-in-code-repositories.md).
Cada nivel solo depende de los de arriba:

| nivel | qué fija | quién lo tiene que leer |
|---|---|---|
| **0 · principios** | qué es una función para el producto | todos |
| **1 · el modelo** | el documento `Function`: identidad, firma, recursos, versión | el compilador, el catálogo, la consola |
| **2 · el contrato** | las seis operaciones y su semántica | cada superficie y cada ejecutor |
| **3 · la plataforma** | las cinco piezas que lo hacen posible | ORE |
| **4 · las superficies** | Python primero; el workspace; Node después | quien programa |
| **5 · lo que se construye encima** | los niveles de abstracción que la base permite | el producto |

Detrás: las decisiones (aprobadas y pendientes), lo que hay hoy (medido), cómo se construye, qué se
acepta a cambio y el estado del arte.

## La visión

Una persona crea un repositorio *functions* en `ventas`, y escribe:

```python
from dataclasses import dataclass
from ore import function, tabla, modelo

clasificador = modelo("ventas.default.qwen3-vl")        # un recurso declarado

@dataclass
class Riesgo:
    nivel: str
    motivo: str
    importe: float

@function
def riesgo_de_cliente(cliente_id: str, umbral: float = 100.0) -> Riesgo:
    pedidos = tabla("bq.ventas.pedidos").where(cliente=cliente_id)
    total = sum(p["total"] for p in pedidos)
    if total <= umbral:
        return Riesgo("bajo", "por debajo del umbral", total)
    dicho = clasificador.pide(f"¿Qué riesgo ves en estos pedidos? {pedidos[:20]}")
    return Riesgo("alto", dicho, total)
```

Y ocurre esto:

- En el panel **Functions** del workspace aparece `riesgo_de_cliente(cliente_id: str, umbral: float
  = 100.0) -> Riesgo`. *Live preview* la ejecuta con los parámetros que escriba, **sin commit**,
  en su rama, contra los datos de verdad.
- Al hacer commit, el catálogo tiene un `Function` `ventas.default.riesgo_de_cliente` con su
  firma, sus recursos (`bq.ventas.pedidos`, `ventas.default.qwen3-vl`) y su linaje. Nadie lo
  escribió a mano: lo extrajo `ore` del código, y el compilador comprueba que coinciden.
- Al etiquetar `1.0.0`, queda **publicada**. Si mañana quita `umbral`, etiquetar `1.1.0` no pasa:
  es incompatible, y es `2.0.0` o nada.
- Se invoca igual desde la consola, desde otra función, desde un pipeline o **desde fuera**, por una
  aplicación con su cliente OAuth:
  `POST /funciones/ventas/default/riesgo_de_cliente/invocar {"parametros": {"cliente_id": "c-42"}, "version": "^1"}`
  → `{"valor": {"nivel": "alto", …}}`.
- Corre en la celda de `ventas`. Su red solo alcanza lo que declaró: la copia de `pedidos` y el
  gateway, porque nombró un modelo. Una función que no nombra ningún modelo no llega al gateway.

## Nivel 0 · Los principios

1. **Una función es una firma.** Entradas con nombre y tipo, una salida con tipo. Todo cuelga de
   ella: la API, el SDK de quien la consume, las versiones, la interfaz y la detección de lo
   incompatible.
2. **El código es la fuente; el árbol, el registro.** La firma se escribe una vez, en el código. El
   documento `Function` del catálogo se deriva y se coteja; no se mantiene a mano en paralelo.
3. **Un contrato de ejecución, varios despliegues.** Bajo demanda, residente o imagen propia son
   configuración, no tres caminos. Lo que se construya encima no sabe en cuál corre.
4. **Lo publicado tiene versión.** Lo que otros consumen está fijado; lo incompatible no se cuela
   como menor.
5. **Solo se alcanza lo declarado.** Datos, modelos, otras funciones y, más adelante, sistemas
   externos. Lo declarado genera los tipos del SDK **y** abre la red; lo no declarado no existe.
6. **Mismo contrato dentro y fuera.** La consola no tiene un camino privado: invoca por la misma API
   que una aplicación externa.
7. **Se mantiene de antes:** corre en la celda ([`0029`](0029-donde-corre-una-funcion.md) ①), lee
   la copia y nunca el origen, y la escritura se propone ([`functions.md`](../functions.md) §1).

## Nivel 1 · El modelo

### La identidad

`<base>.<schema>.<nombre>`, la misma que un `Model` ([`0041`](0041-el-modelo-tiene-sitio.md)) y que
el resto del catálogo: única por schema, nombre sin puntos. **No hay un `apiName` aparte**: el nombre
del catálogo es el nombre de la API. Un repositorio vive en `packages/<base>/<carpeta>`, y sus
funciones se publican en el schema que diga su manifiesto (por defecto, `default`).

### El documento `Function`

Los campos de hoy se mantienen (`runtime`, `model`, `prompt`, `over`, `output`, `effects`,
`authorization`). Se añaden, **derivados del código** cuando `runtime` es de código:

| campo | qué es |
|---|---|
| `entrypoint` | `<ruta del módulo>:<función>`, dentro del repositorio |
| `signature.inputs` | lista ordenada de `{name, type, required, default}` |
| `signature.output` | un tipo |
| `uses` | los recursos declarados: tablas y vistas, modelos, funciones (y, después, fuentes externas) |
| `kind` de la función | `lectura` hoy; `edicion` reservado para la especificación de la escritura |

Los tipos son los de OOS ([`0032`](0032-el-contrato-de-tipos.md)), incluidos los compuestos de
v1alpha17 (`Struct`, `List`, `MediaRef`): un `dataclass` o `TypedDict` es un `Struct`; `list[T]`,
`List<T>`; `Optional[T]`, no requerido. Lo que no tiene tipo en OOS no compila (no hay `Any`).

`runtime: model` sigue siendo una función **sin código**: su firma es la de hoy (`over` → `output`).
Encaja en el mismo modelo como una función declarativa.

### El registro

El árbol **es** el registro: el documento `Function` en la rama principal del repositorio, y una
**etiqueta** `fn/<base>.<schema>.<nombre>@X.Y.Z` sobre el commit que la publica. No hay una base de
datos de funciones aparte; listar versiones es leer etiquetas.

## Nivel 2 · El contrato

Seis operaciones, iguales para cualquier ejecutor y cualquier superficie:

| operación | qué hace | semántica |
|---|---|---|
| **describir** | firma, recursos, versiones publicadas, última corrida | lectura del árbol; sin ejecutar nada |
| **previsualizar** | ejecuta el código **de la rama, sin versión**, con parámetros | solo para quien edita; nunca la consume otro |
| **publicar** | etiqueta una versión | rechazada si la firma rompe respecto de la anterior del mismo mayor |
| **invocar** | `{parametros, version?}` → `{valor}` o `{error}` | parámetros validados contra la firma **antes** de encolar (422); salida validada contra la firma al volver; síncrona con plazo, y si lo pasa, devuelve un trabajo que se consulta |
| **consumir** | otra función o un pipeline la llama | por la misma invocación, con su versión fijada; el SDK la ofrece tipada |
| **retirar** | deja de aceptar invocaciones nuevas de una versión | lo que la consume con esa versión fijada lo dice, con 409 al retirar |

Los errores son valores: una invocación que falla devuelve `{error: {tipo, mensaje}}` y queda en el
informe de la corrida; no tumba al llamante.

## Nivel 3 · La plataforma: las cinco piezas

### ① La firma es del código y se coteja en el árbol

`@function` en el SDK `ore` (Python primero). `ore` extrae la firma y los recursos del código, igual
que ya genera `cedarschema` desde el paquete, y el compilador **compara** lo extraído con el
documento `Function`: si no coinciden, es un diagnóstico, y el commit no pasa la puerta de «no
empeorar». La consola puede escribir el documento por la persona; quien manda es el código.

- **Descartado:** el YAML escrito a mano que el código implementa (dos fuentes que divergen); un
  registro de firmas fuera del árbol (otra verdad, sin historia ni revisión).
- **A cambio:** el extractor es un analizador de Python (y luego de TypeScript) dentro de `ore`, sin
  ejecutar el código: solo lee anotaciones. Lo que no se puede leer sin ejecutar no es firma.

### ② El ejecutor: un protocolo, tres modos

El agente del puesto ya es un ejecutor que **pide trabajo y devuelve resultados**: `GET
/puestos/{id}/pendiente` → `POST /puestos/{id}/celdas/{n}/salida`. Se generaliza a un contrato
neutral al lenguaje: pedir `{id, funcion, version, parametros, testigo}` (204 si no hay nada), ejecutar
y devolver `{valor | error, ms}`. Es la forma del cliente de los *Compute Modules* de Foundry. Con él:

| modo | qué es | cuándo |
|---|---|---|
| **bajo demanda** | un trabajo del puesto ([`0031`](0031-el-puesto.md)) por invocación; ~70 s medidos hoy | ahora (R1) |
| **residente** | réplicas del ejecutor con el código cargado, escaladas por carga (mín. 0 o 1) | cuando una latencia medida lo pida (R5) |
| **módulo** | la imagen del cliente, que habla el mismo protocolo | cuando haga falta un entorno propio (R5) |

**La enmienda a 0029:** Python y Node corren en el puesto, no en wasm. El aislamiento es la red
cerrada de la celda más ⑤, el techo de la clase (no escribe) y el código fijado por commit. wasm
queda como el modo estricto, sin sockets, para quien lo exija.

- **Descartado:** un camino por modo (tres veces la misma lógica de encolar, validar e informar);
  ejecutar en `ore-serve` (0029 ①).
- **A cambio:** el contrato tiene que versionarse desde el principio, porque lo hablarán imágenes que
  no controlamos.

### ③ Versión y registro

SemVer por etiqueta, sobre la rama principal del repositorio. **Incompatible**: quitar o reordenar
una entrada, añadir una obligatoria, cambiar el tipo de salida o borrar la función. **Compatible**:
añadir una entrada opcional, cambiar la implementación. `0.y.z` no promete nada. Quien consume fija
una versión exacta o un rango; un rango se resuelve a la mayor versión publicada que encaje.

- **Descartado:** «siempre la última» (las *query functions* de Foundry, que obligan a un nombre
  nuevo por cada mayor).
- **A cambio:** publicar es un acto aparte del commit, y la consola tiene que hacerlo visible (como
  *Tag version* en la extensión de VS Code de Foundry).

### ④ Invocar con parámetros, desde dentro y desde fuera

`POST /funciones/{b}/{s}/{n}/invocar` acepta `{parametros, version}`. El sujeto es una **persona**,
un **agente** de celda o una **aplicación** (un cliente OAuth del IdP de ORE,
[`0048`](0048-la-identidad-es-de-ore.md)). Antes de encolar se pregunta si puede
([`0047`](0047-el-acceso.md) `puede`); al terminar se dice lo que hizo (`hizo`). El uso de cómputo
y de modelo se cuenta por celda, como ya hace el gateway con los tokens.

- **Descartado:** una API solo para la consola y otra para fuera; un `apiName` distinto del nombre.
- **A cambio:** desde el primer día la invocación es una superficie pública: límites, plazo y
  formato de error son contrato.

### ⑤ Los recursos declarados: el SDK y la red

Lo que la función declara (`uses`, extraído en ①) hace dos cosas:

1. **El SDK tipado:** stubs (`.pyi` para pyright, `.d.ts` para Monaco) con las tablas, los modelos y
   las funciones que usa, para que el editor sepa de qué se habla.
2. **La red:** la `NetworkPolicy` del ejecutor se **deriva** de lo declarado. Un modelo abre el
   gateway con el token del agente de la celda; datos y funciones van por `ore-serve`; lo no
   declarado no tiene ruta. Hoy (M3) el puesto no llega al gateway: `salida-al-modelo` solo
   selecciona `rol: driver`.

- **Descartado:** abrir `salida-al-modelo` a todos los puestos (abre la red también a lo interactivo
  que no lo pidió).
- **A cambio:** declarar un recurso cambia la red del ejecutor, así que una sesión interactiva tiene
  que conocer los `uses` del repositorio antes de arrancar.

### Lo reservado

- **La escritura:** una función de `edicion` **devuelve** una `Propuesta` como tipo de salida
  ([`functions.md`](../functions.md) F1–F5); se aplica por una acción. Su especificación es aparte, y
  ① ya deja el `kind` previsto.
- **Interfaces de función:** una firma sin implementación que varias funciones cumplen (en Foundry,
  `ChatCompletion`). Cabe en ① sin cambios: es un documento con `signature` y sin `entrypoint`.
- **Fuentes externas:** un recurso más en ⑤, que abre su salida y trae su credencial del cofre.

## Nivel 4 · Las superficies

### Python (primero)

- `@function` sobre una función de módulo, con anotaciones; `dataclass`/`TypedDict` para `Struct`.
- `tabla(...)`, `modelo(...)`, `funcion(...)`: los recursos, declarados al nombrarlos en el ámbito
  del módulo; el extractor los lee.
- `modelo(ref).pide(...)` y un cliente compatible con OpenAI, con el token del agente que se renueva
  solo ([`0049`](0049-media-paradigms-in-code-repositories.md) D3).
- Pruebas: `pytest` en el repositorio, como comprobación del commit.

### El workspace

El panel **Functions** con *Live preview* (la rama) y *Published* (las versiones), entradas a mano
y salida tipada; el panel de recursos para declarar lo que se usa; y las acciones del repositorio:
commit, proponer, **etiquetar versión**, actualizar la clase. Es lo que da la extensión de VS Code de
Foundry, sobre nuestro Monaco y nuestro puesto.

### Node y TypeScript (después)

El mismo documento, el mismo contrato, el mismo ejecutor con la imagen `node` que el puesto ya tiene;
la firma, extraída de los tipos de TypeScript.

## Nivel 5 · Lo que se construye encima

Lo que la base permite **sin cambiar las piezas**:

| nivel | sobre qué pieza | qué añade |
|---|---|---|
| módulos de cómputo consultables desde fuera | ② módulo + ④ | la imagen del cliente detrás de la misma API |
| acciones y edición | ① `edicion` + la escritura | la propuesta aplicada por una acción |
| funciones en pipelines | ④ consumir | una función como paso de un transform |
| lógica sin código (tipo AIP Logic) | ① | un editor que **produce** un `Function` con firma; se ejecuta igual |
| aplicaciones | ④ + ③ | consumen versiones fijadas con su cliente OAuth |
| GPU y despliegue residente | ② residente + perfiles de [`0027`](0027-el-modelo-vive-en-el-arbol.md) | réplicas con recursos |

## Las decisiones

| # | decisión | estado |
|---|---|---|
| A1 | el producto es *Code Repositories · Functions*; este documento es su marco | **aprobada** (2026-10-01) |
| A2 | Python es el fundamento; Node entra después por el mismo molde | **aprobada** (2026-10-01) |
| A3 | la escritura tiene su propia especificación; aquí solo se reserva el hueco | **aprobada** (2026-10-01) |
| A4 | las cinco piezas ①–⑤ son la base; el approach acotado es la primera rebanada (R1), no un camino aparte | **aprobada** (2026-10-01) |
| P1 | la fuente de la firma es el código (`@function`), y el documento se deriva y se coteja (①) | **por aprobar** · recomendada |
| P2 | sin `apiName`: el nombre del catálogo es el de la API | **por aprobar** |
| P3 | la enmienda a 0029: Python/Node en el puesto; wasm como modo estricto | **por aprobar** |
| P4 | el plazo de la invocación síncrona antes de pasar a trabajo (propuesta: 60 s, como Foundry) | **por aprobar** |

## Lo que hay hoy (medido el 2026-10-01)

| | estado |
|---|---|
| la clase `functions-python` ([`clases.rs`](../../crates/ore-core/src/clases.rs)) | Python, ejecuta, **no escribe**; plantilla v4: `pyproject.toml` y `funciones/ejemplo.py`, sin `Function` |
| invocar | solo `runtime: model`; el resto, 422 («wasm es F4b»); `effects`/`authorization`, 422 |
| **M1** · una `Function` dentro de un repositorio | el compilador la ve (recorre todo el árbol); `GET /funciones` e `/invocar` **no** (solo `packages/<p>/functions/` y `<schema>/functions/` con `schema.yaml`) |
| **M2** · un trabajo | es un puesto con una celda (la misma plantilla). Medido en `victor`: **68 s** hasta el agente vivo, 42 de ellos esperando nodo. Hay copias al día para `over()` (`bq.ventas.pedidos`, …). La ejecución sobre datos, por medir |
| **M3** · Python → modelo | identidad **sí** (el puesto ya tiene el token del agente de la celda, el mismo que usa la invocación); red **no** (`salida-al-modelo` solo para `rol: driver`) |
| `victor` | ningún repositorio *functions* ni `Function` todavía |
| consola | Run sobre un YAML de `Function` llama a `/invocar`; «Create › Function» del catálogo sin implementar |

## Cómo se construye, y cómo entra

| rebanada | qué | piezas |
|---|---|---|
| **R0 · el contrato** | este documento; la gramática de `signature`/`uses`/`entrypoint` en OOS; casos de conformidad | niveles 1–2 |
| **R1 · la brecha** | Python, bajo demanda, lectura: `@function` y el extractor; `funciones_de` desde el paquete compilado (M1); invocar con parámetros por un trabajo; el informe en `resultados/` y la respuesta; la plantilla `functions-python` v5 con la pareja código + `Function`; Live preview en el workspace; prueba de fuego | ① ② ④ mínimos |
| **R2 · el modelo como recurso** | `modelo(ref)` en el SDK; la red derivada de `uses` (cierra M3) | ⑤ |
| **R3 · las versiones** | etiquetar, la regla de incompatibilidad en el compilador, rangos al consumir | ③ |
| **R4 · fuera** | aplicaciones con cliente OAuth, invocación síncrona con plazo, uso contado | ④ |
| **R5 · residente y módulo** | el protocolo del ejecutor versionado, réplicas, imagen del cliente | ② |
| **R6 · Node** | la misma base en TypeScript | ① ② |

R1 es la **definición de listo** del entorno Python bajo demanda: una persona crea el repositorio,
escribe una función con firma, la previsualiza, hace commit, la ve en el catálogo y la invoca con
parámetros, y una con `effects` sigue dando 422.

## Lo que no se hace aquí

- La escritura (aparte, [`functions.md`](../functions.md)).
- La GPU como decisión de coste (0049 D7).
- Un mercado de funciones entre inquilinos.

## Qué se acepta a cambio

- **Un extractor de firmas** por lenguaje dentro de `ore`.
- **~70 s por invocación** mientras solo exista el modo bajo demanda; parecido a los 100–112 s que
  0029 ya aceptó para `runtime: model`.
- **La red del ejecutor deja de ser fija**: depende de lo que el repositorio declara.
- **Un contrato público** (la invocación y el protocolo del ejecutor) que hay que versionar desde
  el primer día.

## Anexo · El estado del arte (Foundry, resumido)

De `palantir.com/docs/foundry`, leído el 2026-10-01:

- **Tres lenguajes** con plantilla propia (TS v1, TS v2, Python). Python y TS v2 tienen modelos de
  lenguaje, ejecución desplegada e interfaces (TS v2); solo Python se llama desde Pipeline Builder
  (`/functions/language-feature-support/`).
- **Firma explícita**: todo argumento y retorno con tipo; Python con `@function` de `functions.api`
  (`/functions/python-getting-started/`, `/functions/types-reference/`).
- **Imports de recursos** (object types, query functions, modelos, fuentes, interfaces) que generan
  un SDK tipado; la red, cerrada por defecto, se abre con una fuente (`/functions/resource-imports-sidebar/`,
  `/functions/api-calls/`).
- **Ciclo**: *live preview* sin commit (280 s), commit, *Tag version*, registro; SemVer con la lista
  de lo incompatible; consumidores con versión o rango (`/functions/functions-versioning/`,
  `/functions/version-range-dependencies-for-functions/`).
- **Modos**: *serverless* (por ejecución, varias versiones a la vez) y *deployed* (contenedor
  residente, réplicas, GPU, una versión) (`/functions/functions-deployed/`,
  `/functions/python-functions-deployed/`). Límite por defecto: 60 s.
- **Fuera**: `POST /api/v2/ontologies/{o}/queries/{apiName}/execute {parameters}` → `{value}`, y el
  OSDK; la aplicación tiene que estar dada de alta (`/api/v2/ontologies-v2-resources/queries/execute-query/`,
  `/functions/permissions/`).
- **Compute Modules**: la imagen del cliente con un cliente que pide trabajo (`GET job` → 200/204,
  `POST results/{jobId}`), modos función y pipeline, escalado por carga hasta cero, red cero por
  defecto (`/compute-modules/overview/`, `/compute-modules/advanced-custom-client`,
  `/compute-modules/scaling`).
- **Interfaces de función** (`ChatCompletion`) y **AIP Logic** como productores de funciones sin
  código (`/functions/function-interfaces/`, `/logic/overview/`).
