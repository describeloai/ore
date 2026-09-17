# 0029 · Dónde corre una función

**Estado:** propuesto (medido el 2026-09-17; escrito el 2026-09-18) · **Fecha:** 2026-09-18 · **Decide:** que el
código de una `Function` corre **en la celda del inquilino**, en su pool privado y con su
identidad, **nunca en `ore-serve` ni en un servicio central**; que tiene **dos modos** con el
mismo delegado —**bajo demanda**, un `Job` por invocación desde la cola, que es la figura que
ya existe, y **residente**, un `ore-invoke` con sesión por celda, cuando la latencia de una
`Action` lo pida y se haya medido—; que el aislamiento es **la red cerrada más WASI sin
sockets**, no un sandbox de sistema; que el puente con el modelo es la misma función por el
gateway; y **el orden de las etapas** hasta que una función esté lista para producción, con
[`functions.md` F6](../functions.md#f6--la-definición-de-listo) como definición de listo. Aplica
la naturaleza de [`oos/spec/v1alpha10`](../../vendor/oos/spec/v1alpha10/00-scope.md).

---

## El problema

v1alpha10 dejó escrito **qué** es una función —lógica encapsulada que lee, edita o infiere sobre
la copia, con su superficie en los dos sentidos— y el compilador lo lee. Lo que nadie ha decidido
es **dónde corre el código y cómo llega a los datos**. Hoy no corre en ningún sitio:
[`functions.md` §2](../functions.md#2-qué-hay-y-qué-falta-medido) lo mide —la `Propuesta`, el
cotejo y la regla de integridad existen; **quien invoca (F4) y quien aplica (F5) no**—, y lo único
que ha ejecutado una función de verdad fue un `Job` lanzado a mano en E0 de
[`0027`](0027-el-modelo-vive-en-el-arbol.md).

La pregunta tiene dos respuestas posibles y hay que elegir una antes de escribir el delegado:
**centralizado** (un servicio de funciones de la plataforma, como hace Foundry) o **dedicado por
celda** (el código corre donde corre el catálogo y la copia de ese inquilino).

## Lo que se miró antes de decidir

**La celda, tal como está.** Un inquilino es un namespace `t-<n>` en `ore-mesh`
([`0024`](0024-donde-corre-el-inquilino.md)). El único sitio de la plataforma donde corre código
que toca datos es un `Job` que Flux crea desde la cola y Kueue aterriza en el pool privado
`jobs-p`: nodos sin Cloud NAT, `NetworkPolicy` que abre DNS, metadatos, las APIs de Google por
443, el 5432 del origen y el gateway de modelos (`MODELOS/32:8000`,
[`0027` E0–E2](0027-el-modelo-vive-en-el-arbol.md)). Imagen `ore-drivers`; identidad, el agente
de la celda. Así corren el catálogo (`malla/44`) y la copia (`malla/48`), con tres mitades:
traer el testigo, hacer lo que toca el origen, publicar al árbol. `ore-serve` no ejecuta nada
de eso: lleva `ore` sin TLS y `git`, y es el plano de control
([`0020`](0020-el-plano-de-control.md)). Un `Job` tarda **100–112 s** en arrancar (E0).

**Lo que E0 demostró, a mano.** Un `Job` de `victor` ejecutó `functions/segmentar.yaml`: llamó al
gateway con el token de la celda, produjo la `Propuesta`, `ore verify` la cotejó
(«cae dentro de lo que el paquete autoriza») y quedó en `propuestas/` con commit. Todo lo que
una función necesita alcanzar —la forja, el gateway, y el bucket cuando exista— lo alcanza ese
pod y **solo** ese pod.

**Cómo lo hacen los demás** (fuentes públicas, 2026-09-17):

| | dónde corre | modos | aislamiento | acceso al dato |
|---|---|---|---|---|
| **Foundry** | en la plataforma de Palantir | *serverless* (bajo demanda, cualquier versión, 1 GiB por defecto hasta 5) y *deployed* (contenedor de larga vida, una versión); *compute modules* para contenedor propio con sidecar | el del proceso Node/Python en su infraestructura | por el OSDK, nunca por conexión |
| **Snowflake** | en la cuenta del cliente | UDF/procedimientos en sandbox dentro del warehouse; Snowpark Container Services para contenedores y jobs | gVisor, reconstruido para multi-tenant | el warehouse |
| **Databricks** | plano de control de Databricks | funciones de Unity Catalog en *serverless* (Spark Connect remoto); Model Serving; Lakebase para escribir | Lakeguard | por el catálogo |

Tres cosas comparten: **nadie ejecuta la función junto al dato por conexión** (la función recibe
filas por un SDK o un plano de datos); **todos tienen dos modos de latencia**, bajo demanda y
residente; y **el aislamiento es parte del diseño**, no de la operación. Y una diferencia: los
tres centralizan la ejecución en su plataforma. ORE ya decidió lo contrario para todo lo demás en
[`0024`](0024-donde-corre-el-inquilino.md).

**Lo que falta debajo.** Una función lee y escribe **la copia**, y la copia en la celda no existe
todavía: nadie declara `materialized`, ningún inquilino tiene bucket, y el `Job` de copia no se
ha lanzado nunca en la malla ([`0027` P1](0027-el-modelo-vive-en-el-arbol.md), I1–I4). GCS por
S3/HMAC lo prohíbe la política de la organización (412) y R2 sacaría la copia de la VPC: hace falta
`ore-store-gcs` con Workload Identity.

## La decisión

> ### ① Dedicado por celda. El código corre donde corre el catálogo y la copia de ese inquilino.

En el namespace `t-<n>`, en el pool `jobs-p`, con la identidad del agente de la celda, con la
`NetworkPolicy` que ya tiene. No en `ore-serve` —el plano de control no ejecuta lo que gobierna, y
no alcanza ni el bucket ni el gateway— ni en un servicio central de funciones: lo que Foundry
centraliza nosotros lo repartimos por celda, y el aislamiento no es gVisor sino **la red cerrada
por construcción más WASI sin sockets**. Un módulo que intente abrir un socket falla por no
tener canal, no por una comprobación (F4). Es más fuerte, más barato, y es la frase del producto:
*un agente recibe una superficie, no credenciales*.

> ### ② Dos modos, un delegado. Bajo demanda primero; residente cuando se mida.

**Bajo demanda:** `ore-serve` recibe la invocación en el verbo, decide con Cedar y el sujeto de la
petición si puede, y rinde en la cola `49-la-invocacion.yaml` como hoy rinde el catálogo; Flux
crea el `Job`, Kueue lo aterriza. Es la figura que existe, y vale para una función sobre una
vista entera o una inferencia por lotes.

**Residente:** el mismo `ore-invoke` con una sesión abierta —como `ore-maintain`
([`0013`](0013-el-protocolo-del-mantenedor.md)): la sesión es el estado— como `Deployment` por
celda, en el mismo pool y con la misma política. Es lo que una `Action` desde una pantalla
necesita, y **no se construye hasta que la latencia del `Job` se mida contra una acción real**.
Foundry corre la misma función en los dos modos; aquí también.

**Lo que no entra:** el contenedor propio del cliente (*compute modules*). Rompe el sandbox, y
`runtime: wasm` es a propósito la única forma de traer código.

> ### ③ Las tres mitades del Job, y qué alcanza cada una.

| mitad | qué | alcanza |
|---|---|---|
| **traer** | el árbol en el commit; las filas de `over` y `reads` desde la copia sellada, por el plan de la vista (`ore-store-gcs` lee Parquet) | forja, bucket |
| **invocar** | `ore-invoke`: wasm + WASI 0.2, sin sockets, las filas como entrada, una `Propuesta` como salida. Con `runtime: model`, el mismo delegado llama al gateway con el token de la celda, con `prompt` y filas | gateway (solo con `model`) |
| **verificar y aplicar** | `ore verify` coteja contra `effects` y la clave; la propuesta se aplica sobre la copia como copia sucesora con el recibo de sucesión ([`0017`](0017-la-escritura-sobre-el-sustrato.md) §A, `ore-store-gcs` fundiendo por clave, idempotente por digest); propuesta y recibo se empujan al árbol como hoy el informe de copia | bucket, forja |

**Nunca el origen.** Una función lee la copia y escribe la copia. El origen queda intacto byte a
byte, que es lo que F6 existe para afirmar.

> ### ④ Quién invoca lo decide el plano de control; con qué identidad corre lo decide la celda.

Cedar (`authorization`) se evalúa en `ore-serve` con el sujeto de la petición antes de encolar;
la concesión puede negar y no ensanchar ([`0023`](0023-donde-vive-un-secreto.md)). El `Job`
corre con el agente de la celda, no con el usuario, y la `Propuesta` lleva quién la pidió. El
módulo wasm vive en el árbol (`entrypoint: dist/<f>.wasm`) y `attested` lo ata por digest (F3).

> ### ⑤ `ore-invoke` es un delegado, en `ore-drivers`.

wasmtime es una dependencia grande con FFI y `tests/dependencias.rs` la veta en `ore`
([`0008`](0008-el-protocolo-del-driver.md)). Va en la imagen `ore-drivers`, que es la que corre
en el `Job`, y habla por stdin/stdout como los demás.

## Lo que se acepta a cambio

- **Latencia.** 100–112 s de arranque por invocación mientras solo exista el modo bajo demanda.
  Se acepta porque es medible y porque el modo residente está decidido, no improvisado.
- **Un delegado más** en la imagen, y wasmtime dentro de la VPC. Se acepta porque la
  alternativa —código del cliente en un contenedor propio— es la que rompe la garantía.
- **La copia como prerrequisito.** Sin P1 no hay funciones, y P1 es un peldaño entero.
- **Lo que sigue sin decidir**, y se dice: qué gana cuando el refresco desde el origen contradice
  una edición ([`functions.md` §7.4](../functions.md#74-leer-lo-que-acabas-de-escribir-y-qué-gana-contra-el-origen));
  si el historial de ediciones es un objeto propio o la cadena de copias sucesoras (§7.3); si la
  ontología puede sostener hechos que ningún origen tuvo (§7.5). Los tres se contestan con datos
  de F6, no antes.

## El abordaje — las etapas, en orden, hasta producción

Cada etapa se mide antes y cierra con una prueba de fuego; ninguna pinta lo que no llega. El
orden no es de conveniencia: **cada una es prerrequisito de la siguiente**.

| | etapa | qué | listo cuando |
|---|---|---|---|
| **P1** | **la copia en la celda** ([`0027` P1](0027-el-modelo-vive-en-el-arbol.md)) | `ore-store-gcs` con Workload Identity; bucket por inquilino en el aprovisionamiento; el convergedor lanza `materializar` (`malla/48`); `discover` induce `changes.key` cuando el catálogo la conoce; `standard` ⇒ `materialized` en todas las vistas del paquete | `GET /paquetes/olist` de `demo` dice N filas por entidad y el digest de la copia; releer no lee el origen (`refresco.sh`) |
| **L0** | **la línea base de la celda** | el retículo de eje `integrity`, el conducto `materialization.payload` autorizado y el esquema Cedar regenerado **en el aprovisionamiento** (`gen-inquilino.py` / `ore init`), no como escrituras de Forge. Sale de la medida: cuatro de las siete escrituras de una función no eran de la función | un inquilino nuevo compila con una función v1alpha10 escrita a mano sin tocar nada más |
| **F4** | **`ore-invoke`, bajo demanda** | el delegado (wasm + WASI 0.2, sin sockets; `runtime: model` por el gateway); `49-la-invocacion.yaml` en la cola; `POST /funciones/{ns}/{n}/invocar` en `ore-serve` con Cedar y el sujeto; la `Propuesta` al árbol | un módulo que abre un socket falla por no tener canal; una función de lectura devuelve su `output`; una de edición deja una `Propuesta` que `ore verify` acepta |
| **F2·F3** | **flujo y endosos sobre la propuesta** | `flow` y `governance` sobre los edits propuestos; verificar la atestación **antes** de invocar | una propuesta por debajo de la clasificación de lo leído no compila; un endoso que no verifica no llega a ejecutarse |
| **F5** | **aplicar por la vista** | copia sucesora con recibo de sucesión; fusión por clave; propuesta y recibo en el árbol | aplicar dos veces produce la misma copia y la segunda no escribe; leer la vista después devuelve el valor nuevo |
| **V** | **los verbos de Forge** | `/documentos/Function` y `/documentos/Action` (dos filas más de `KINDS`; `quien_nombra`: `Ruleset.duties.call`, `Action.call`, `effects.writes`); `GET /propuestas`; la consola: Functions, Actions, y el camino cuando falta línea base | Functions y Actions «real N/N» en `medida-forge-contra-serve.py` |
| **F6** | **la definición de listo** | la prueba de fuego con números afirmados sobre `demo`/`olist`: los cinco actos y las cinco negativas de [`functions.md` F6](../functions.md#f6--la-definición-de-listo), y **el origen intacto byte a byte** | los números están afirmados en CI |
| **R** | **el modo residente** | `ore-invoke` con sesión, `Deployment` por celda; `Action` desde la consola | medida la latencia de una acción real contra el `Job`, y solo si la medida lo pide |

Lo que este orden dice y conviene no perder: **la fila de Forge para `Function` (V) va después
de que una función corra (F4)**, no antes. Pintar en la consola un documento que nadie puede
ejecutar es el boceto otra vez.

## El orden, revisado (2026-09-17) — por dependencias de la meta, no por el orden en que se escribió

La meta es **el primer modelo real ejecutando inferencia, de forma consistente, sobre conjuntos
de datos**. Mirado paso a paso —¿es prerrequisito de eso, o de «listo para producción»?— el orden
de arriba cambia en cuatro sitios:

| paso | ¿prerrequisito de la meta? | por qué |
|---|---|---|
| **I5** (P1 cerrada con números) | sí | sin filas en el bucket no hay nada que leer |
| **el modelo real** (Vast g1 para aceptar; la cuota G4 para quedarse) | sí | contra `de-mentira` no hay inferencia; es de Bastion y va en paralelo |
| **L0** | **no** | una propiedad DRAFT sin exigencia de integridad se escribe sin endoso (OOS7002); el conducto ya lo pone `tras_inducir`; Cedar sólo si la función declara `authorization`. Es de F6 |
| **F4 con wasm + WASI** | **no** | el puente al modelo no ejecuta código del cliente. **F4a** (`runtime: model`) primero; **F4b** (wasm) con las funciones de código |
| **la ontología mínima** | **sí, y no estaba** | `effects.writes` exige una `Entity` con clave (OOS2005, OOS2024), y desde 0027 C1 una base estándar nace sin entidades: modelar una tabla pequeña y contestar su `clave` es un paso, y es de la Forge |
| **F2·F3** | no para la primera inferencia | sí para que valga más que `inferred` |
| **F5** | **sí** | sin aplicar, la inferencia es una Propuesta en el árbol, no un hecho que una consulta devuelva |
| **§7.4** (qué gana entre el refresco y un efecto) | **sí, y estaba aparcado** | «consistente» es exactamente que el hecho inferido sobreviva al refresco. La copia se rehace entera desde el origen: sin decidirlo, F5 escribe y el siguiente refresco lo borra. Se decide **en F5**, con un caso real |
| **V**, **R** | no | uso, y latencia |

Y dos cosas de tamaño: una Model Function se aplica **una vez por fila** (`customers` son 99 k
llamadas): la aceptación va sobre una tabla pequeña (`product_category_name_translation`, 71
filas; `sellers`, 3 095) o una vista con `where`.

**El orden que vale:**

1. **I5** — P1 con números en `demo`.
2. **F4a** con una **función de lectura** (`reads` + `output`, sin `effects`): lee la copia, llama al modelo real, devuelve. Sin entidad, sin clave, sin L0, sin wasm. El primer «modelo real infiere sobre datos», con latencia y coste medidos.
3. **La ontología mínima** — una tabla pequeña modelada y su `clave` contestada (Forge).
4. **F4a con `effects`** → Propuesta cotejada por `ore verify`.
5. **F5**, y con ella **§7.4 decidido**: aplicar, refrescar, y que el hecho siga. Aquí se cumple «consistente».
6. Después: **L0**, **F2·F3**, **V**, **F6**, **F4b**, **R**.

> ### F4a · la función de lectura — medido el 2026-09-17, antes de escribir nada

`pruebas-de-fuego/medida-f4a-lectura.py` (20 medidas, 29 s; nada de pago encendido): cuánto de
cada mitad del Job (③) existe ya para una `Function` con `runtime: model`, `over`, `output` y
**sin `effects`** sobre la vista copiada de 71 filas de `demo` (`olist_copia.productCategoryNameTranslation`).

| mitad | lo que hay | lo que falta |
|---|---|---|
| **la gramática** | `ore validate` la admite tal cual sobre una base estándar **sin entidades** (A1–A2); sin `over`·`reads`·`effects` → OOS1004; `model` que no resuelve → OOS2005 (A4–A5) | nada. Dos notas para `oos`: sin `output` también compila (A3: leer y no devolver nada), y `over` sobre una vista **sin copia** compila (A6): es del runtime decirlo, y el Job lo dirá con 409 |
| **traer** | Parquet → filas está (`carga::leer`, es lo que `anterior` usa para fundir); demo tiene 3 artefactos (1,26 MB) y 3 recibos (B2–B3) | un verbo **`leer`** en `ore-store-gcs`: cabecera del plan por stdin, las filas por stdout una por línea. Es `anterior` sacando lo que ya lee |
| **invocar** | la red (`salida-al-modelo` en la plantilla), la identidad (token del agente, E0 c), `GET /modelos/{n}` → `{url, model}` (D6), el cuerpo de la llamada (E0 d: `/v1/chat/completions`, `temperature: 0`, `usage` con los tokens) | **`ore-invoke`** (⑤): en `ore-drivers`, con `ureq` como `ore-store-gcs`; stdin = la puerta, el id, el `prompt`, la forma de `output` y las filas; stdout = una línea por fila `{fila, output, tokens, ms}`; N llamadas a la vez (E0 b midió 4 sin degradar). Y `ore invoke <función>` en `ore-cli`, que encadena `leer → ore-invoke → sellar` sin abrir un socket |
| **devolver** | — (D5: ③ sólo habla de la Propuesta) | **decisión**: el resultado es un artefacto del bucket del inquilino (`ore-store-gcs sellar` con cabecera `{función, commit, digest de la copia leída}`: mismo sobre, mismo nombre por digest, idempotente) y un **informe** en el árbol `resultados/<ns>_<f>_<corrida>.json` con filas, tokens, ms, y una muestra de 5. Los datos no van al árbol; los números sí, como la copia |
| **mandar** | `ore verify` (no aplica a una lectura); el informador y Data › Jobs (un prefijo más: `invocar-`) | **`49-la-invocacion.yaml`** (Job con el agente de la celda; env `FUNCION`, `MODELO_URL`, `MODELO_ID` resueltos al encolar), **`POST /funciones/{ns}/{n}/invocar`** en `ore-serve` (compila el árbol, resuelve el `Model`, 409 si `over` no tiene copia, encola) |
| **el modelo** | `demo` no tiene `Model`; `modelos-e0` está **parada** y `GET /modelos` tarda 7 s en decirlo (C1–C2) | el alta (`POST /modelos`), la máquina (0,01 $/h) y un g1 (E0 b: Vast ~1,5 $/h; la cuota G4 sigue en 0). **Con go** |

**El espectro, en orden:**

| | qué | acepta | paga |
|---|---|---|---|
| **F4a·I1** | `ore-store-gcs leer` | las 71 filas de demo vuelven del bucket **desde un Job de la celda** (con el token del metadata server, como `sellar`), y localmente contra un artefacto sellado en la prueba | no |
| **F4a·I2** | `ore-invoke` + `ore invoke` | `la-invocacion-se-decide.sh` en CI contra un vLLM de mentira en Python (`/v1/chat/completions` que contesta por fila): 71 filas → 71 salidas selladas + informe; una fila que el modelo no contesta sale como `error`, no tumba la corrida | no |
| **F4a·I3** | `49` + `POST /funciones/…/invocar` + prefijo `invocar-` | en demo, contra el stub en `modelos-e0` (**enciende la e2-micro**): el Job aparece en Data › Jobs corriendo, con su log, y termina con el informe en el árbol | 0,01 $/h |
| **F4a·I4** | el modelo real | un g1 en Vast como E0 b (**community**: las 71 categorías de Olist son públicas, y aun así es su decisión), `POST /modelos` en demo, la misma función: latencia por fila, tokens, coste; se destruye con verificación | ~1 $ |

Lo que no entra en F4a: `effects` (es el paso 4), Cedar (sólo si la función declara `authorization`),
wasm, la `Action` (v1alpha10 `02-action`), y el botón en la Forge (se llama por la API y se ve en Jobs).

**F4a·I1 y I2, hechas (2026-09-17).** Decidido: **los datos al bucket, los números al árbol**.
`ore-store-<tipo> leer` devuelve una copia **por su nombre** (el que su informe dejó en
`copias/`), cabecera y filas una por línea: por nombre y no por plan porque «la vigente» la sabe
quien construyó la cabecera, no el almacén. `ore-invoke` (crate nuevo, en `ore-drivers`) es el
delegado de ⑤ para `runtime: model`: petición + filas por stdin, una línea por fila en el mismo
orden, N hilos, `temperature: 0`, y al modelo se le pide un objeto JSON con las claves de
`output`; una fila que no contesta bien sale como `error` y la corrida sigue. `ore invoke
<árbol> --funcion ns.f` encadena `leer → ore-invoke → sellar` sin abrir un socket: el resultado es
un artefacto del mismo almacén (esquema = el de la copia + `output`; `conducto` =
`function:<ns>.<f>`; `testigo.valor` = la clave de la copia leída), así que dos corridas iguales
son el mismo artefacto y la segunda no sube un byte; `--informe DIR` deja
`resultados/<ns>_<f>_<corrida>.json` (filas, ok, errores, tokens, ms, muestra de 5). Se niega:
`over` sin copia declarada o sin hacer, `effects`, `runtime` que no sea `model`, un `output` que
se llama como un campo de la copia, sin puerta. `la-invocacion-se-decide.sh` (CI) lo cierra de
punta a punta contra un S3 y un vLLM de mentira en Python (`de-mentira.py`): 12 filas → 11
selladas + 1 error dicho; sin `MODELO_TOKEN`, 401 por fila y nada que sellar. Lo que I3 trae:
`49-la-invocacion.yaml`, `POST /funciones/{ns}/{n}/invocar`, el prefijo `invocar-` en Jobs.
