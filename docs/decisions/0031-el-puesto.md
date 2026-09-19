# 0031 · El puesto

**Estado:** borrador (escrito el 2026-09-19 tras el estado del arte; la medida se anota al pie) ·
**Fecha:** 2026-09-19 · **Decide:** que W3 —«la sesión viva» de [`0030`](0030-el-arbol-en-el-editor.md)—
no es un kernel Python en un pod sino **el sustrato de ejecución** del que cuelga todo lo que no
es YAML: celdas Python y notebooks, funciones TypeScript y Java, SQL sobre datasets grandes,
inferencia, entrenamiento y fine-tuning; que ese sustrato tiene **una sola unidad, el puesto**
(imagen + identidad de la persona + datos + recursos) con **dos vidas** (sesión y trabajo); que
las dependencias se **declaran en el árbol y las resuelve CI** en una capa sobre imágenes base
**numeradas**; que el código lee y escribe datos **por un SDK con alias** y nunca por el
almacén; que un modelo es **un documento del árbol** con los pesos en el bucket; que la red del
puesto está **cerrada** y se abre por política con nombre; y que la cola gobierna las sesiones
con **prioridad alta y sin tomar prestado**. Sigue a
[`docs/investigacion/w3-estado-del-arte.md`](../investigacion/w3-estado-del-arte.md) y a
[`0029`](0029-donde-corre-una-funcion.md) (dónde corre una función).

## El problema

W0–W2 hicieron del workspace **el sitio** donde una organización lee, escribe, compila, ejecuta
(vistas) y propone su ontología. Todo lo que corre ahí hoy es YAML: `Run` sobre una `View` es el
motor de proyección de `ore-drivers` sobre el Parquet de la copia; una `Function` de inferencia es
un Job (`ore-invoke`, 0029 F4a). Lo que la organización quiere hacer **en el mismo sitio** es lo
que hace en cualquier otro entorno de datos: una celda Python que lee `hr.empleados` como
DataFrame; una función TypeScript que la consola invoca; un SQL que agrega 200 M de filas; adaptar
un modelo existente para que infiera sobre un conjunto; entrenar; fine-tunear. Todo a escala
enterprise, y todo **sin que ni una fila salga de la celda**.

Si se construye para la celda Python y luego se estira a lo demás, se paga tres veces. La
pregunta de 0031 es qué hay que fijar **antes** para que cada cosa de esa lista sea «otra imagen
y otra vida» y no «otro sistema».

## Lo mirado

[`w3-estado-del-arte.md`](../investigacion/w3-estado-del-arte.md): Foundry (Code Workspaces,
Code Repositories, Functions, compute modules), Databricks (serverless, base environments,
GPU), Snowflake (Container Runtime), SageMaker (spaces; Kueue en HyperPod), Colab Enterprise,
Kubeflow Workspaces. Nadie hace algo estructuralmente distinto de lo que teníamos escrito; lo
que hacen todos y no teníamos está abajo con su nombre.

## Decisión

### 1 · La unidad: el puesto, con dos vidas

Un **puesto** es *imagen + identidad de la persona + datos + recursos*. Existe en dos vidas:

| | **sesión** | **trabajo** |
|---|---|---|
| qué | viva, con estado, **una persona**, un nodo | un Job de Kueue que corre una cosa y muere |
| para | el notebook, el REPL, probar, mirar | una invocación, un entrenamiento, una compilación, un SQL grande |
| vida | TTL de inactividad (**30 min**) y tope (**24 h**) | `activeDeadlineSeconds` por clase de trabajo |
| escala | **no crece**: la sesión es de un nodo (Foundry, Snowflake) | lo grande se encola; el sabor decide el nodo |
| hoy | — | `49-la-invocacion.yaml`, `48-la-copia.yaml` **ya son puestos-trabajo** |

Una celda Python, una función TS, un fine-tune y un SQL son **lo mismo con distinta imagen y
distinta vida**. Lo que no cambia entre ellos: el `initContainer` que trae el testigo, la cuenta
`driver` con Workload Identity, el emptyDir de trabajo, `deny-all` con sus excepciones con
nombre, la cola `cola` del inquilino.

### 2 · Un agente en el pod, y el código habla con el SDK

Dentro de cada puesto corre **el agente** (un sidecar nuestro, como el de Foundry: 0,25 vCPU):
obtiene y renueva el token de la persona (RFC 8693, `act` = el puesto), resuelve alias de datos
contra `ore-serve`, sube salidas al bucket y **hace polling de trabajo** en vez de aceptar
conexiones — casa con «sin entrada» y con el patrón *compute module*. El código del cliente no
sabe de pods, tokens ni buckets: importa `ore` y llama al SDK.

### 3 · Runtimes: imágenes base numeradas + una capa por paquete, resuelta en CI

- **Pocas imágenes base por lenguaje, numeradas**: `puesto-python:1`, `puesto-node:1`,
  `puesto-jvm:1`, con su `requirements`/lock **dentro de la imagen** (Databricks: entornos 1…5;
  Foundry: `hawk.lock`). Nunca `latest`. Lo que una celda importa hoy importa igual en un año.
- **Las dependencias se declaran en el árbol** (`pyproject.toml`, `package.json`, `pom.xml` del
  paquete) y **CI las resuelve y construye una capa** sobre la base: `puesto-python:1+<digest del
  lock>`. Cambiar una librería es un cambio de código: pasa por commit, PR y checks.
- `pip install` en la celda puede existir como **comodidad sin persistencia** (Snowflake) y sólo
  si la política de red lo abre; nunca es la verdad del entorno.
- `puesto-python:1` existe desde hoy (`Dockerfile` etapa 6, `cloudbuild.yaml`): los nodos de
  `jobs-p` no alcanzan Docker Hub, así que todo lo que corre ahí sale de nuestro registro.

### 4 · Datos: un plano (Arrow/Parquet en el bucket), un SDK con alias, y fallback de rama

- **Un plano de datos**: las copias (W1) ya son Parquet en el bucket del inquilino con sobre
  `ORECOPY1`. Datasets, salidas de trabajos y pesos de modelos van al mismo sitio, particionados,
  con su documento en el árbol. Interactivo = DuckDB/DataFusion en el puesto; masivo = un
  trabajo (motor distribuido cuando se mida; el contrato no cambia).
- **El SDK con alias**: `over("hr.empleados")` resuelve por la ontología a la copia vigente, con
  la identidad de la persona y **la potestad de `ore-iam`** (una vista que no puede leer, no
  llega al DataFrame). El código nunca ve el bucket ni una credencial.
- **Fallback de rama** (Foundry): una rama lee las copias de `main` mientras no tenga las suyas;
  una vista `materialized` nueva en la rama se copia en la rama.
- **Escribir es publicar una salida con nombre**: un dataset, una tabla, un modelo, una función
  — nunca ficheros sueltos. Es lo que hace que el linaje exista.

### 5 · Cómputo: sabores, cuotas y la política de sesiones

- Ya es Kueue: `ResourceFlavor jobs` (etiqueta, no pool), `ClusterQueue` por inquilino (10 CPU ·
  36 Gi hoy), `LocalQueue cola`. Se añaden **sabores `gpu` y `spot`** como etiquetas; el pool
  aparece cuando se mida y se pague; la política no se reescribe.
- **Sesiones**: prioridad alta (100 frente a 75 entrenamiento, 50 evaluación, 25 batch),
  **prestar cuota sí, tomar prestada no** (una sesión nunca corre sobre recursos reclamables),
  *preemption* dentro de la organización. Es la guía de Kueue para *interactive spaces*.
- **GPU concedida por cola**, no por pedirla: sólo un inquilino con cuota GPU puede pedir el sabor.

### 6 · Modelos: documentos del árbol, pesos en el bucket

Un kind `Model` (base, adaptador, versión, artefactos en el bucket, la `Function` que lo sirve).
Entrenar o fine-tunear es un **trabajo** que produce una versión; inferir es la `Function` de
0029 (`ore-invoke`). El registro de modelos es **git + bucket**, no una pieza nueva. **Publicar
desde la celda** es un snippet del SDK (`ore.models.publish(...)`), y el modelo está disponible al
instante para el resto (Foundry).

### 7 · Red: cerrada, con nombre, y sólo lectura sin exportación

- `deny-all` sigue; cada apertura es una `NetworkPolicy` con nombre (`salida-al-…`), como hoy.
  Un puesto alcanza: DNS, metadatos, el bucket (Google APIs), `ore-serve`, y **nada más** por
  defecto. Internet, sólo por política nombrada del inquilino (repos privados, un origen).
- **Modo *restricted outputs***: una sesión sobre datos marcados puede leer y **no puede
  publicar ni sacar**; se decide por política del inquilino sobre el dataset.

### 8 · El contrato consola ↔ puesto

`ore-serve` es la puerta: `POST /puestos` (abrir una sesión: imagen, sabor, rama),
`POST /puestos/{id}/ejecutar` (una celda: texto, lenguaje) → salida **tipada** (tabla, texto,
imagen, job, error) al panel de resultados que ya existe; `DELETE` cierra. El editor no sabe de
pods. Los trabajos van por la cola como hoy (Flux rinde el Job) y la consola los ve en Data › Jobs.

## Los peldaños de W3

| | qué | acepta |
|---|---|---|
| **W3.0** | medir el puesto (`medida-w3-el-puesto.py`) y esta decisión | los números de abajo |
| **W3.1** | la sesión Python: `puesto-python:1`, el agente, `POST /puestos`, una celda → salida al panel; TTL y tope | una celda lee `over("…")` como DataFrame en la consola y no alcanza internet |
| **W3.2** | dependencias del árbol → capa en CI; `pyproject` del paquete | un paquete declara `polars` y la sesión lo importa sin `pip` |
| **W3.3** | trabajos desde la sesión: SQL grande y un entrenamiento como Job encolado con sabor | un `count(*)` sobre 200 M de filas vuelve como job con su salida en el bucket |
| **W3.4** | TS y JVM: `puesto-node:1`, `puesto-jvm:1`; una función TS invocable desde la consola | la función del árbol contesta en la consola con la identidad de la persona |
| **W3.5** | `Model`: publicar desde la celda, fine-tune como trabajo con sabor `gpu` | un adaptador entrenado en la celda sirve por `Function` |

## Lo medido (`pruebas-de-fuego/medida-w3-el-puesto.py`, victor)

*(se rellena al correr la medida)*

## Lo que se aparca

- El motor distribuido para lo masivo (Ray/Spark sobre la cola): el contrato (Parquet en el
  bucket + documento en el árbol + trabajo encolado) lo admite; se elige cuando haya un dataset
  que no quepa en un nodo.
- `restricted.googleapis.com` (el /30 con Cloud DNS privado): el paso 2 de `20-driver.yaml`.
- Sesiones compartidas (dos personas en el mismo notebook): SageMaker lo tiene; no antes de W3.1.
