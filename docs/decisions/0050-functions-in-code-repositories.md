# 0050 · Functions in code repositories

**Estado:** aceptado (las decisiones, 2026-10-01) · R1 en construcción: **P1 hecho** (el contrato,
OOS v1alpha18, oos `fd1ed1a`) y **P2 hecho** (`ore-core` habla v1alpha18: 17/17 y `ore diff` por
parámetro, 27/27) y **P3 hecho** (invocar: las funciones del paquete compilado, los
parámetros contra `input`, el trabajo del puesto con el arnés y `resultados/`) y **P4 hecho**
(`ore.modelo(ref)` acotado a `models`, y `salida-al-modelo-de-una-funcion`: la etiqueta
`ore.dev/usa-modelo` que solo pone `ore-serve` al trabajo de una función con `models`); P5–P6 por
hacer. La regla de red llega a un inquilino vivo al convergerlo. **Decide:** qué es el **producto** *Functions* de los code repositories, y que
se construye **sobre lo que ORE ya define** —la gramática de `Function` (OOS v1alpha10–14), el
puesto y su promoción ([`0031`](0031-el-puesto.md) W3.8), la taxonomía de versiones de OOS
(`91-versioning`), la red por política con nombre (0031 §7), el acceso
([`0047`](0047-el-acceso.md))— sin piezas paralelas. Lo nuevo es poco y está dicho: `runtime: python`
en la gramática, `models` como recurso que una función de código declara, el lector de firmas de
Python en el compilador, y la invocación con parámetros. **Ejecuta** la enmienda a
[`0029`](0029-donde-corre-una-funcion.md) que 0031 W3.8 dejó escrita.

## La pregunta

¿Qué tiene que ser verdad para que un repositorio de clase *functions* sea **el mejor sitio para
construir lógica encapsulada** sobre los conjuntos de datos del catálogo y sobre los modelos, y para
que encima se construyan **niveles de abstracción** (acciones, lógica sin código, aplicaciones,
funciones consultables desde fuera) **sin volver a escalar la base**?

La respuesta: **casi todo está**. ORE se proyectó para esto: una `Function` ya es lógica encapsulada
con superficie tipada, el puesto ya ejecuta Python, Node y Java, la versión ya la decide `ore diff`.
Lo que falta es **unirlo**: que el código de un repositorio se promueva a una `Function` que se
invoca. Este documento fija ese marco y el orden.

## Cómo se lee

| nivel | qué fija | apoyado en |
|---|---|---|
| **0 · principios** | qué es una función para el producto | OOS v1alpha10 `01-function` §1 |
| **1 · el modelo** | el documento, su identidad y su versión | OOS v1alpha10–14, `91-versioning` |
| **2 · el contrato** | las operaciones | 0029, 0031, 0047 |
| **3 · la plataforma** | las cinco piezas, cada una con su teoría | 0029, 0031, 0036, 0047, 0048 |
| **4 · las superficies** | Python primero; el workspace; Node después | 0031 W3.4, 0036 |
| **5 · lo que va encima** | los niveles de abstracción | — |

## La visión

Una persona tiene un repositorio *functions* en `ventas` y escribe en `funciones/riesgo.py`:

```python
from ore import tabla, modelo

def riesgo(cliente, umbral):
    pedidos = tabla("ventas.pedidos").where(cliente=cliente["clienteId"])
    total = sum(p["importe"] for p in pedidos)
    if total <= umbral:
        return {"nivel": "bajo", "total": total}
    dicho = modelo("ventas.extractor").pide(f"¿Riesgo en estos pedidos? {pedidos[:20]}")
    return {"nivel": dicho, "total": total}
```

y su contrato, en `functions/riesgo.yaml`:

```yaml
apiVersion: oos.dev/v1alpha18
kind: Function
metadata: { name: riesgo, namespace: ventas }
spec:
  runtime: python
  entrypoint: funciones/riesgo.py:riesgo
  over: ventas.clientes
  reads: [ventas.pedidos]
  models: [modelo/ventas.extractor]
  input:
    umbral: { type: Decimal, required: true }
  output:
    nivel: { type: String }
    total: { type: Decimal }
  limits: { timeout: 60s }
```

Y ocurre esto:

- El compilador comprueba que `funciones/riesgo.py` define `riesgo`, que sus parámetros son la fila
  y `umbral`, y que `ventas.pedidos` y el modelo existen. Si no, no compila.
- En el workspace, *Live preview* la ejecuta en su rama, sin versión, con un `umbral` escrito a mano.
- Al hacer commit, el catálogo tiene `ventas.riesgo` con su superficie y su linaje.
- Al publicar el paquete, `ore diff` decide el salto: quitar `umbral` es mayor y no pasa como menor.
- Se invoca con parámetros (`POST /funciones/ventas/riesgo/invocar {"parametros": {"umbral": 100}}`)
  desde la consola, desde otra función o por API. Corre en la celda, en un trabajo del puesto,
  y su red solo alcanza la copia y, porque declaró un modelo, el gateway.

## Nivel 0 · Los principios

1. **Una función es lógica encapsulada con superficie tipada.** Es la naturaleza de OOS v1alpha10
   §1: lo que puede leer es la unión de las vistas que declara, lo que devuelve es su `output`, lo
   que causa es la unión de sus `effects`. Nada fuera.
2. **El documento es el contrato; el código lo implementa.** El árbol es el sistema de registro
   ([`0018`](0018-la-ontologia-es-el-sistema-de-registro.md)). Pasar de código a función es una
   **promoción explícita** (0031 W3.8): la herramienta puede escribir el documento a partir del
   código una vez, pero el documento no se regenera solo, y el compilador comprueba que el código lo
   cumple.
3. **Se ejecuta en el puesto, en la celda.** El puesto es el sustrato de ejecución (0031); la
   función corre en la celda y nunca en `ore-serve` (0029 ①).
4. **Lo publicado tiene versión, y la decide `ore diff`.** SemVer del paquete con la taxonomía de
   `91-versioning`; no hay versiones por función aparte.
5. **Solo se alcanza lo declarado.** `over`/`reads` para los datos, `models` para los modelos; la red
   se abre por política con nombre (0031 §7).
6. **La escritura se propone** ([`functions.md`](../functions.md)) y tiene su propia especificación.

## Nivel 1 · El modelo

### El documento

La `Function` de OOS, sin campos nuevos para la firma: **`input` y `output` ya son la firma**
(`$defs/parameters`, un mapa nombre → `{type, required, description}` con el sistema de tipos de la
versión). Lo que v1alpha18 añade:

| | qué | por qué |
|---|---|---|
| `runtime: python` | el código es un `def` de Python | la promoción de 0031 W3.8; `node` y `jvm` entran después por la misma regla |
| `entrypoint: <ruta>.py:<def>` | dentro del paquete, sin salir de él | lo que `wasm` ya exigía, con la función nombrada |
| `models: [modelo/<ref>]` | los modelos que el código **puede** llamar | `model` sigue siendo «el modelo es lo que se ejecuta» (`runtime: model`); una función de código lo **usa**, y lo usado se declara |
| la firma del `def` | los parámetros son la fila (si hay `over`) y las claves de `input` | el compilador lo coteja sin ejecutar nada |
| `limits.timeout` | el plazo de la invocación | ya estaba en la gramática |

### La identidad

`<base>.<schema>.<nombre>`, como todo el catálogo ([`0038`](0038-los-tres-niveles.md),
[`0041`](0041-el-modelo-tiene-sitio.md)). **No hay `apiName`**: el nombre del catálogo es el de la
API.

### La versión

La del paquete, con `ore diff` (`91-versioning` §5–6): un cambio que rompe al consumidor exige
mayor, y `ore diff` falla si la versión declarada no corresponde (`OOS5021`). Quien consume fija con
`ontology.lock` (`range: "^2.1"`). Sobre una función: quitar un parámetro es `OOS5001`, un parámetro
obligatorio nuevo `OOS5003`, cambiarle el tipo `OOS5002`/`OOS5010`; añadir uno opcional es menor.
**Medido:** `ore diff` hoy no ve el cambio de un parámetro (`diff.rs` lee `input.type` como si
`input` fuera un tipo); se arregla en R1.

## Nivel 2 · El contrato

| operación | qué hace | apoyado en |
|---|---|---|
| **describir** | superficie, recursos, última corrida | `GET /funciones` |
| **previsualizar** | el código de la rama, sin publicar, con parámetros, en la sesión | el puesto (0031 W3.1) |
| **invocar** | `{parametros}` → `{valor}` o `{error}`, validado contra `input` antes de encolar y contra `output` al volver; dentro de `limits.timeout`, y si no, un trabajo que se consulta | `POST …/invocar` (0029 F4a), el trabajo (0031) |
| **consumir** | otra función o un pipeline la llama por la misma invocación | — |
| **publicar** | publicar el paquete | `ore pack` + `ore-registry` |
| **retirar** | `OOS5007` sin anunciarlo en el manifiesto | `91-versioning` |

Quién invoca: persona o agente de celda, preguntado a `ore-acceso` (`puede`, 0047) antes de
encolar; `authorization` (Cedar) sigue en 422 hasta que se evalúe (0029 ④). Una **aplicación**
externa con su cliente OAuth no está modelada todavía (0048 la deja como «un cliente más» del IdP):
es R4.

## Nivel 3 · La plataforma: las cinco piezas

| # | pieza | la teoría que la sostiene | lo que falta |
|---|---|---|---|
| **①** | **la superficie es el documento** | OOS `Function` (`input`, `output`, `over`, `reads`); 0031 W3.8 (promoción explícita, el decorador como material) | `runtime: python`, `entrypoint` con `def`, `models` (v1alpha18); el lector de firmas de Python en `ore-core` (hoy solo SQL se analiza: `sql_del_arbol`) |
| **②** | **el ejecutor es el puesto** | 0031: una unidad, dos vidas (sesión y trabajo); 0029: bajo demanda primero, residente cuando se mida contra una acción real | el arnés que llama al `def` con la fila y los parámetros; `funciones_de` desde el paquete compilado (M1) |
| **③** | **la versión es la del paquete** | `91-versioning` §5–6, `ore diff`, `ontology.lock`, `ore-registry` | que `ore diff` vea cada parámetro |
| **④** | **invocar con parámetros** | 0029 F4a (`/invocar`, la cola, `resultados/`); 0047 `puede` | validar `parametros` contra `input`; respuesta síncrona dentro de `limits.timeout`; aplicaciones (R4) |
| **⑤** | **lo declarado abre la red** | 0031 §7 (`salida-al-…`), y la capa de dependencias (`pyproject.toml`, 0031 W3.2) | que el trabajo de una función con `models` lleve la etiqueta que `salida-al-modelo` selecciona (hoy solo `rol: driver`, M3) |

**La enmienda a 0029, tal como 0031 W3.8 la escribió:** (1) el aislamiento de una función de código
pasa de «red cerrada **más** WASI sin sockets» a **la red cerrada**: un `def` abre sockets, y que no
lleguen a nada lo garantiza la `NetworkPolicy`, la misma garantía con la que ya corren los puestos;
(2) «`runtime: wasm` es la única forma de traer código» sigue valiendo **contra el contenedor del
cliente**: una imagen nuestra que corre código del cliente no lo es. wasm queda como el modo estricto.

**Lo que este marco deja fuera, y por qué:** el contenedor propio del cliente (los *compute modules*
de Foundry). 0029 lo rechaza porque rompe el sandbox. Una función **nuestra** consultable desde
fuera sí cabe (④ + R4); una imagen del cliente es otra decisión, explícita, si llega.

## Nivel 4 · Las superficies

- **Python (primero).** El `def` de módulo; `tabla()`/`over()` y `modelo()` del SDK, acotados a lo
  declarado como `@transform` ya acota a sus `inputs` (0031 W3.7 ⑤); la plantilla `functions-python`
  v5 nace con la pareja `functions/<f>.yaml` + `funciones/<f>.py`.
- **El workspace.** El panel *Functions* con *Live preview* (la rama) y *Published* (el paquete
  publicado), entradas a mano y salida tipada; «Promote to function» que escribe el documento desde el
  código una vez. Es lo que da la extensión de VS Code de Foundry, sobre Monaco y el puesto.
- **Node y Java (después).** `runtime: node|jvm` por la misma regla; los intérpretes ya están (0031
  W3.4).

## Nivel 5 · Lo que se construye encima

| nivel | sobre qué | qué añade |
|---|---|---|
| acciones y edición | ① `effects` + [`functions.md`](../functions.md) | la propuesta aplicada por una acción |
| funciones en pipelines | ④ consumir | una función como paso de un transform |
| lógica sin código | ① | un editor que **produce** el documento; se ejecuta igual |
| funciones consultables desde fuera | ④ + R4 | aplicaciones con su cliente, versión fijada por `ontology.lock` |
| residente y GPU | ② + perfiles de [`0027`](0027-el-modelo-vive-en-el-arbol.md) | réplicas cuando la latencia medida lo pida |

## Las decisiones

| # | decisión | estado |
|---|---|---|
| A1 | el producto es *Code Repositories · Functions*; este documento es su marco | aprobada · 2026-10-01 |
| A2 | Python es el fundamento; Node y Java después, por la misma regla | aprobada · 2026-10-01 |
| A3 | la escritura tiene su propia especificación | aprobada · 2026-10-01 |
| A4 | la base es lo que ORE ya define; R1 es la primera rebanada, no un camino aparte | aprobada · 2026-10-01 |
| D1 | el documento es el contrato; el código lo implementa; promoción explícita (0031 W3.8) | cerrada · 2026-10-01 |
| D2 | sin `apiName`: el nombre del catálogo | cerrada · 2026-10-01 |
| D3 | la enmienda a 0029, la de 0031 W3.8; el contenedor del cliente, fuera | cerrada · 2026-10-01 |
| D4 | el plazo es `limits.timeout` | cerrada · 2026-10-01 |
| D5 | la versión es la del paquete, por `ore diff` | cerrada · 2026-10-01 |
| D6 | la red, por política con nombre seleccionada por lo declarado | cerrada · 2026-10-01 |

## Lo que hay hoy (medido el 2026-10-01)

| | estado |
|---|---|
| la gramática | `Function` en v1alpha14: `runtime: wasm\|model`, `input`/`output` tipados, `over`/`reads`/`effects`, `limits`; el compilador **no aplica** el enum (`runtime: python` compila hoy, 0031 W3.8) |
| la clase `functions-python` | ejecuta, no escribe; plantilla v4 sin `Function` |
| invocar | solo `runtime: model`; `authorization`, 422 |
| **M1** | una `Function` dentro de un repositorio la ve el compilador, no `GET /funciones` ni `/invocar` |
| **M2** | un trabajo es un puesto con una celda: 68 s en `victor` (42 esperando nodo); hay copias al día para `over()` |
| **M3** | el puesto tiene el token del agente de la celda; la red no llega al gateway |
| `ore diff` | no ve el cambio de un parámetro |

## Cómo se construye

**R1 · Python bajo demanda, de lectura** — la brecha:

| paso | qué | dónde |
|---|---|---|
| **P1 · el contrato** | OOS v1alpha18: `runtime: python`, `entrypoint` con `def`, `models`, la firma del `def`, `limits.timeout`, la versión por parámetro; casos de conformidad | `oos` |
| **P2 · el compilador** | `ore-core` habla v1alpha18: la forma, el lector de firmas de Python, `models` resueltos, `ore diff` por parámetro | `ore-core` |
| **P3 · invocar** | `funciones_de` desde el paquete compilado; `parametros` contra `input`; el trabajo del puesto con el arnés; el informe en `resultados/`; `limits.timeout` | `ore-serve`, `puesto/python` |
| **P4 · el modelo desde Python** | `modelo(ref)` en el SDK, acotado a `models`; la etiqueta de red del trabajo | SDK, `malla` |
| **P5 · la plantilla** | `functions-python` v5: la pareja documento + código | `clases.rs` |
| **P6 · la prueba y la consola** | `pruebas-de-fuego/la-funcion-python.sh`; Live preview y Run en el workspace | ORE, consola |

Después: **R2** residente (cuando se mida), **R3** Node y Java, **R4** aplicaciones externas, y la
escritura por su especificación.

## Qué se acepta a cambio

- **Un lector de firmas de Python** en `ore-core`, sin ejecutar código: solo la cabecera del `def`.
- **~70 s por invocación** mientras solo haya bajo demanda; parecido a lo que 0029 aceptó para
  `runtime: model`.
- **El aislamiento de una función de código es la red cerrada**, no WASI (0031 W3.8).

## Anexo · Foundry, lo que se tomó y lo que no

De `palantir.com/docs/foundry`, leído el 2026-10-01. **Se toma:** la firma tipada como centro
(`/functions/types-reference/`), *live preview* sin publicar, la lista de lo incompatible
(`/functions/functions-versioning/`), los recursos importados que generan tipos y abren la red
(`/functions/resource-imports-sidebar/`, `/functions/api-calls/`), la invocación por API con
parámetros (`/api/v2/ontologies-v2-resources/queries/execute-query/`) y el panel de la extensión de VS
Code (`/functions/navigating-vscode/`). **No se toma:** el registro de funciones fuera del código
fuente de verdad (aquí es el árbol), el `apiName` aparte, las versiones por función (aquí, la del
paquete) y el contenedor del cliente (*compute modules*, `/compute-modules/overview/`), que 0029
rechaza.
